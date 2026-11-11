mod db;
mod platform;
mod settings;
mod taps;
mod translate;

use db::{fnv1a, now_ms, text_hash, Db, ExportFile, Filter, Item, NewItem, SourceApp, Tag};
use settings::Settings;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const PANEL: &str = "panel";
/// 最初に出すときのパネルの高さ（中サイズ 1 段）。実際の高さはフロント側から resize_panel で合わせる
const PANEL_HEIGHT: f64 = 300.0;
/// パネルの高さの下限（小サイズ 1 段より少し低め）
const MIN_PANEL_HEIGHT: f64 = 240.0;
/// いちばん大きいカード 1 段ぶんの高さ（カード + 間隔）。上限の計算に使う
const ROW_HEIGHT: f64 = 252.0;
/// 多段表示の日付見出し 1 つぶんの高さ（見出し 20 + 間隔 12）
const DAY_HEADER_HEIGHT: f64 = 32.0;
const PANEL_MARGIN: f64 = 12.0;

#[derive(Clone, Copy, Debug)]
enum Action {
    TogglePanel,
    Screenshot,
    Ocr,
}

pub struct AppState {
    db: Mutex<Db>,
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    data_dir: PathBuf,
    /// 自分で貼り付け中（自分の書き込みを履歴に拾わない）
    pasting: AtomicBool,
    /// 記録を一時停止中
    paused: AtomicBool,
    /// 「続けて貼る」の残り（⌘V / Ctrl+V のたびに次をクリップボードへ）
    queue: Mutex<std::collections::VecDeque<(i64, Content)>>,
    last_count: AtomicI64,
    /// この時刻(ms)までに来た画像はスクショ扱い
    screenshot_until: AtomicI64,
    /// いま表示しているパネルの高さ（論理px, f64 のビット列）
    panel_height_now: std::sync::atomic::AtomicU64,
    shortcuts: Mutex<HashMap<u32, Action>>,
    /// 修飾キーの 2 回押しで動く操作
    double_taps: Mutex<HashMap<taps::Mod, Action>>,
}

type Res<T> = Result<T, String>;
fn e<E: ToString>(err: E) -> String {
    err.to_string()
}


fn remove_files(files: Vec<String>) {
    for f in files {
        let _ = std::fs::remove_file(f);
    }
}

// ───────────────────────── クリップボード監視 ─────────────────────────

fn start_monitor(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        state.last_count.store(platform::change_count(), Ordering::SeqCst);
        let mut tried_icons: HashSet<String> = HashSet::new();
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let count = platform::change_count();
            if count == state.last_count.swap(count, Ordering::SeqCst) {
                continue;
            }
            if state.pasting.load(Ordering::SeqCst) || platform::is_concealed() {
                continue;
            }
            if state.paused.load(Ordering::SeqCst) {
                continue;
            }
            // コピー元はできるだけ早く取る（ユーザーがアプリを切り替える前に）
            let src = platform::source_app();
            if src.as_ref().is_some_and(|a| is_excluded(&state, &a.id)) {
                continue;
            }
            // 自分以外がコピーしたら「続けて貼る」は終わり
            state.queue.lock().unwrap().clear();
            let shot = now_ms() < state.screenshot_until.load(Ordering::SeqCst)
                || platform::screenshot_ui_running();
            if let Err(err) = capture(&app, &state, src, shot, &mut tried_icons) {
                log::warn!("capture failed: {err}");
            }
        }
    });
}

fn is_excluded(state: &AppState, app_id: &str) -> bool {
    state.settings.lock().unwrap().excluded_apps.iter().any(|x| x.eq_ignore_ascii_case(app_id))
}

/// OS 標準スクショの保存先を見張り、新しいスクショを履歴とクリップボードに入れる
fn start_screenshot_watcher(app: AppHandle) {
    let Some(dir) = platform::screenshot_dir() else { return };
    let prefixes = platform::screenshot_prefixes();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let started = std::time::SystemTime::now();
        loop {
            std::thread::sleep(Duration::from_millis(700));
            if !state.settings.lock().unwrap().watch_screenshots {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                // 書き込み中は ".スクリーンショット…" という隠しファイルになっている
                if name.starts_with('.') || !prefixes.iter().any(|p| name.starts_with(p.as_str())) {
                    continue;
                }
                let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                if !matches!(ext.as_str(), "png" | "jpg" | "jpeg") || seen.contains(&path) {
                    continue;
                }
                let modified = entry.metadata().and_then(|m| m.modified()).ok();
                if modified.is_none_or(|m| m < started) {
                    seen.insert(path);
                    continue;
                }
                seen.insert(path.clone());
                // 履歴にはクリップボード監視経由で「スクリーンショット」として入る
                state
                    .screenshot_until
                    .store(now_ms() + 3_000, Ordering::SeqCst);
                if let Err(err) = write_clipboard(&Content::Image(path)) {
                    log::warn!("screenshot import failed: {err}");
                }
            }
        }
    });
}

fn icon_for(state: &AppState, app: &platform::AppInfo, tried: &mut HashSet<String>) -> Option<String> {
    let safe: String = app
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' })
        .collect();
    let out = state.data_dir.join("icons").join(format!("{safe}.png"));
    if out.exists() {
        return Some(out.to_string_lossy().into());
    }
    if tried.insert(app.id.clone()) && platform::save_app_icon(app, &out) {
        return Some(out.to_string_lossy().into());
    }
    None
}

/// 他のアプリがクリップボードを開いている間は読めないことがあるので少し待って再試行する
fn open_clipboard() -> Res<arboard::Clipboard> {
    let mut last = String::new();
    for i in 0..5 {
        match arboard::Clipboard::new() {
            Ok(cb) => return Ok(cb),
            Err(err) => last = err.to_string(),
        }
        std::thread::sleep(Duration::from_millis(30 * (i + 1)));
    }
    Err(last)
}

fn capture(
    app: &AppHandle,
    state: &AppState,
    src: Option<platform::AppInfo>,
    shot: bool,
    tried: &mut HashSet<String>,
) -> Res<()> {
    // ファイル（Finder / エクスプローラーでコピー）はファイル名の文字より先に見る
    if let Some(files) = platform::read_files() {
        return capture_files(app, state, src, tried, &files);
    }
    let mut cb = open_clipboard()?;

    let text = match cb.get_text() {
        Ok(t) => Ok(t),
        Err(arboard::Error::ClipboardOccupied) => {
            std::thread::sleep(Duration::from_millis(80));
            cb.get_text()
        }
        Err(err) => Err(err),
    };
    if let Ok(text) = text {
        if !text.trim().is_empty() {
            let icon = src.as_ref().and_then(|s| icon_for(state, s, tried));
            let hash = text_hash(&text);
            let item = NewItem {
                kind: "text",
                text: Some(&text),
                image_path: None,
                thumb_path: None,
                width: None,
                height: None,
                hash: &hash,
                source_name: src.as_ref().map(|s| s.name.as_str()),
                source_id: src.as_ref().map(|s| s.id.as_str()),
                source_icon: icon.as_deref(),
            };
            return finish_insert(app, state, &item);
        }
    }

    let img = match cb.get_image() {
        Ok(img) => img,
        Err(_) => return Ok(()), // ファイルなど未対応の形式
    };
    let (w, h) = (img.width as u32, img.height as u32);
    let hash = format!("i:{}", fnv1a(&img.bytes));
    let dir = state.data_dir.join("images");
    std::fs::create_dir_all(&dir).map_err(e)?;
    let file = dir.join(format!("{}.png", &hash[2..]));
    let thumb = dir.join(format!("{}.thumb.png", &hash[2..]));
    if !file.exists() {
        let rgba = image::RgbaImage::from_raw(w, h, img.bytes.into_owned()).ok_or("bad image")?;
        rgba.save(&file).map_err(e)?;
        let (tw, th) = fit(w, h, 480, 300);
        image::imageops::thumbnail(&rgba, tw, th).save(&thumb).map_err(e)?;
    }

    let (sname, sid, sicon) = if shot {
        state.screenshot_until.store(0, Ordering::SeqCst);
        (Some("スクリーンショット".to_string()), Some("clipo.screenshot".to_string()), None)
    } else {
        let icon = src.as_ref().and_then(|s| icon_for(state, s, tried));
        (src.as_ref().map(|s| s.name.clone()), src.as_ref().map(|s| s.id.clone()), icon)
    };
    let (fp, tp) = (file.to_string_lossy().to_string(), thumb.to_string_lossy().to_string());
    let item = NewItem {
        kind: "image",
        text: None,
        image_path: Some(&fp),
        thumb_path: Some(&tp),
        width: Some(w as i64),
        height: Some(h as i64),
        hash: &hash,
        source_name: sname.as_deref(),
        source_id: sid.as_deref(),
        source_icon: sicon.as_deref(),
    };
    finish_insert(app, state, &item)
}

/// コピーされたファイルを「ファイル」カードとして記録（中身はパスを改行でつないだもの）
fn capture_files(
    app: &AppHandle,
    state: &AppState,
    src: Option<platform::AppInfo>,
    tried: &mut HashSet<String>,
    files: &[String],
) -> Res<()> {
    let text = files.join("\n");
    let hash = format!("f:{}", fnv1a(text.as_bytes()));
    // 1 つ目のファイルのアイコン（Finder と同じもの）をサムネイルにする
    let dir = state.data_dir.join("icons");
    let thumb = dir.join(format!("file-{}.png", &hash[2..]));
    let thumb = (thumb.exists() || platform::save_file_icon(&files[0], &thumb))
        .then(|| thumb.to_string_lossy().to_string());
    let icon = src.as_ref().and_then(|s| icon_for(state, s, tried));
    let item = NewItem {
        kind: "file",
        text: Some(&text),
        image_path: None,
        thumb_path: thumb.as_deref(),
        width: None,
        height: None,
        hash: &hash,
        source_name: src.as_ref().map(|s| s.name.as_str()),
        source_id: src.as_ref().map(|s| s.id.as_str()),
        source_icon: icon.as_deref(),
    };
    finish_insert(app, state, &item)
}

fn fit(w: u32, h: u32, mw: u32, mh: u32) -> (u32, u32) {
    let s = (mw as f64 / w as f64).min(mh as f64 / h as f64).min(1.0);
    (((w as f64 * s) as u32).max(1), ((h as f64 * s) as u32).max(1))
}

fn finish_insert(app: &AppHandle, state: &AppState, item: &NewItem) -> Res<()> {
    let max = state.settings.lock().unwrap().max_history;
    let files = {
        let db = state.db.lock().unwrap();
        db.insert_or_bump(item).map_err(e)?;
        db.prune(max).map_err(e)?
    };
    remove_files(files);
    let _ = app.emit("clips-changed", ());
    Ok(())
}

// ───────────────────────── パネル表示 ─────────────────────────

fn show_panel(app: &AppHandle) {
    let Some(win) = app.get_webview_window(PANEL) else { return };
    platform::remember_target();

    // マウスカーソルのあるモニタの下部に表示
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| win.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        // 前回の高さで出し、中身に合わせてフロント側から resize_panel で調整する
        let h = f64::from_bits(app.state::<AppState>().panel_height_now.load(Ordering::SeqCst));
        place_panel(&win, &m, h);
    }
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit_to(PANEL, "panel-shown", ());
}

/// モニタの作業領域の下端にパネルを置く（logical_h は論理ピクセルの高さ）
fn place_panel(win: &tauri::WebviewWindow, m: &tauri::Monitor, logical_h: f64) {
    let wa = m.work_area();
    let scale = m.scale_factor();
    let margin = (PANEL_MARGIN * scale) as i32;
    let logical_h = logical_h.max(MIN_PANEL_HEIGHT);
    let height = ((logical_h * scale) as u32).min(wa.size.height.saturating_sub(2 * margin as u32));
    let width = wa.size.width.saturating_sub(2 * margin as u32);
    let _ = win.set_size(PhysicalSize::new(width, height));
    let _ = win.set_position(PhysicalPosition::new(
        wa.position.x + margin,
        wa.position.y + wa.size.height as i32 - height as i32 - margin,
    ));
}

/// 表示する中身に合わせてパネルの高さを変える（設定の最大段数ぶん＋日付見出し1つが上限）
#[tauri::command]
fn resize_panel(app: AppHandle, state: State<AppState>, height: f64) {
    let max_rows = state.settings.lock().unwrap().panel_rows.max(1) as f64;
    // カードの大きさで高さが変わるので、下限・上限はゆるめに取る
    let max_h = 96.0 + max_rows * ROW_HEIGHT + DAY_HEADER_HEIGHT;
    let h = height.round().clamp(MIN_PANEL_HEIGHT, max_h);
    if f64::from_bits(state.panel_height_now.swap(h.to_bits(), Ordering::SeqCst)) == h {
        return;
    }
    if let Some(win) = app.get_webview_window(PANEL) {
        if let Ok(Some(m)) = win.current_monitor() {
            place_panel(&win, &m, h);
        }
    }
}

fn hide_panel_inner(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(PANEL) {
        let _ = win.hide();
    }
}

fn toggle_panel(app: &AppHandle) {
    match app.get_webview_window(PANEL) {
        Some(w) if w.is_visible().unwrap_or(false) => {
            hide_panel_inner(app);
            platform::activate_target();
        }
        Some(_) => show_panel(app),
        None => {}
    }
}

fn start_screenshot(app: &AppHandle) {
    hide_panel_inner(app);
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        state.screenshot_until.store(i64::MAX, Ordering::SeqCst);
        platform::run_screenshot();
        state
            .screenshot_until
            .store(now_ms() + platform::SCREENSHOT_GRACE_MS, Ordering::SeqCst);
    });
}

/// 範囲を選んで文字起こしし、テキストとして履歴とクリップボードに入れる
fn start_ocr(app: &AppHandle) {
    hide_panel_inner(app);
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        state.pasting.store(true, Ordering::SeqCst);
        let tmp = std::env::temp_dir().join("clipo-ocr.png");
        let result = platform::capture_and_ocr(&tmp).and_then(|text| {
            let Some(text) = text else { return Ok(false) };
            if text.trim().is_empty() {
                return Err("文字が見つかりませんでした".into());
            }
            write_clipboard(&Content::Text(text.clone()))?;
            let hash = text_hash(&text);
            finish_insert(
                &app,
                &state,
                &NewItem {
                    kind: "text",
                    text: Some(&text),
                    image_path: None,
                    thumb_path: None,
                    width: None,
                    height: None,
                    hash: &hash,
                    source_name: Some("文字起こし"),
                    source_id: Some("clipo.ocr"),
                    source_icon: None,
                },
            )?;
            Ok(true)
        });
        std::thread::sleep(Duration::from_millis(300));
        state.last_count.store(platform::change_count(), Ordering::SeqCst);
        state.pasting.store(false, Ordering::SeqCst);
        match result {
            Ok(true) => {
                let _ = app.emit("ocr-done", ());
            }
            Ok(false) => {}
            Err(err) => {
                log::warn!("ocr failed: {err}");
                let _ = app.emit("app-error", format!("文字起こしに失敗しました: {err}"));
            }
        }
    });
}

fn open_settings_window(app: &AppHandle) {
    hide_panel_inner(app);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("Clipo 設定")
        .inner_size(560.0, 640.0)
        .resizable(true)
        .center()
        .build()
        .map(|w| w.set_focus());
}

// ───────────────────────── ショートカット ─────────────────────────

/// 登録に失敗したショートカットの説明を返す
fn register_shortcuts(app: &AppHandle, s: &Settings) -> Vec<String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let state = app.state::<AppState>();
    let mut map = state.shortcuts.lock().unwrap();
    map.clear();
    let mut taps_map = state.double_taps.lock().unwrap();
    taps_map.clear();
    let mut errors = vec![];
    let mut wanted = vec![
        ("パネル表示", s.panel_shortcut.clone(), Action::TogglePanel),
        ("文字起こし", s.ocr_shortcut.clone(), Action::Ocr),
    ];
    if s.screenshot_enabled {
        wanted.push(("範囲スクショ", s.screenshot_shortcut.clone(), Action::Screenshot));
    }
    for (label, key, action) in wanted {
        if key.trim().is_empty() {
            continue;
        }
        if key.starts_with(taps::PREFIX) {
            match taps::Mod::parse(&key) {
                Some(m) if taps_map.contains_key(&m) => {
                    errors.push(format!("{label}: 他のショートカットと重複しています"));
                }
                Some(m) => {
                    taps_map.insert(m, action);
                    start_tap_watch(app);
                }
                None => errors.push(format!("{label}: {key} は無効なキーです")),
            }
            continue;
        }
        match Shortcut::from_str(&key) {
            Ok(sc) => {
                if map.contains_key(&sc.id()) {
                    errors.push(format!("{label}: 他のショートカットと重複しています"));
                    continue;
                }
                match gs.register(sc) {
                    Ok(_) => {
                        map.insert(sc.id(), action);
                    }
                    Err(err) => errors.push(format!(
                        "{label}: {key} を登録できません（他のアプリが使用中の可能性）: {err}"
                    )),
                }
            }
            Err(err) => errors.push(format!("{label}: {key} は無効なキーです: {err}")),
        }
    }
    errors
}

fn run_action(app: &AppHandle, action: Action) {
    match action {
        Action::TogglePanel => toggle_panel(app),
        Action::Screenshot => start_screenshot(app),
        Action::Ocr => start_ocr(app),
    }
}

/// 修飾キーの 2 回押しの監視を始める（必要になったときに 1 度だけ）
fn start_tap_watch(app: &AppHandle) {
    let handle = app.clone();
    let started = taps::start(move |m| {
        let action = handle.state::<AppState>().double_taps.lock().unwrap().get(&m).copied();
        if let Some(action) = action {
            let h = handle.clone();
            let _ = handle.run_on_main_thread(move || run_action(&h, action));
        }
    });
    let h2 = app.clone();
    taps::on_paste(move || on_user_paste(&h2));
    if started {
        // macOS の NSEvent 監視はメインスレッドで登録する必要がある
        let _ = app.run_on_main_thread(platform::start_modifier_watch);
    }
}

// ───────────────────────── 貼り付け ─────────────────────────

fn send_paste_keys() -> Res<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings as ES};
    let mut en = Enigo::new(&ES::default()).map_err(e)?;
    #[cfg(target_os = "macos")]
    let (modifier, v) = (Key::Meta, Key::Other(9)); // kVK_ANSI_V
    #[cfg(windows)]
    let (modifier, v) = (Key::Control, Key::Other(0x56)); // VK_V
    #[cfg(not(any(target_os = "macos", windows)))]
    let (modifier, v) = (Key::Control, Key::Unicode('v'));
    en.key(modifier, Direction::Press).map_err(e)?;
    en.key(v, Direction::Click).map_err(e)?;
    en.key(modifier, Direction::Release).map_err(e)?;
    Ok(())
}

/// カーソルを左へ n 文字動かす（{カーソル} の位置へ戻す）
fn send_left_keys(n: usize) -> Res<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings as ES};
    let mut en = Enigo::new(&ES::default()).map_err(e)?;
    for _ in 0..n.min(5000) {
        en.key(Key::LeftArrow, Direction::Click).map_err(e)?;
    }
    Ok(())
}

enum Content {
    Text(String),
    Files(Vec<String>),
    Image(PathBuf),
}

fn write_clipboard(c: &Content) -> Res<()> {
    let mut cb = open_clipboard()?;
    match c {
        Content::Text(t) => cb.set_text(t.clone()).map_err(e),
        Content::Files(paths) => {
            drop(cb);
            platform::write_files(paths)
        }
        Content::Image(p) => {
            let img = image::open(p).map_err(e)?.to_rgba8();
            let (w, h) = img.dimensions();
            cb.set_image(arboard::ImageData {
                width: w as usize,
                height: h as usize,
                bytes: img.into_raw().into(),
            })
            .map_err(e)
        }
    }
}

/// 貼り付け時に置き換える値（日時はパネル側のローカル時刻）
#[derive(serde::Deserialize, Default)]
struct PasteVars {
    date: String,
    time: String,
}

/// 貼ったあとのカーソル位置の目印（私用領域の文字）
const CURSOR_MARK: char = '\u{E000}';

/// 保存したテキストの差し込み記号を置き換える。{カーソル} は目印に変える
fn expand_vars(text: &str, vars: &PasteVars, clip: &mut Option<Option<String>>) -> String {
    let mut out = text.replace("{日付}", &vars.date).replace("{時刻}", &vars.time);
    if out.contains("{クリップボード}") {
        let c = clip.get_or_insert_with(|| open_clipboard().ok().and_then(|mut cb| cb.get_text().ok()));
        out = out.replace("{クリップボード}", c.as_deref().unwrap_or(""));
    }
    out.replace("{カーソル}", &CURSOR_MARK.to_string())
}

/// 貼る中身と、貼ったあとカーソルを左へ戻す文字数
/// as_path=true ならファイルをフルパスの文字として貼る
fn load_contents(
    state: &AppState,
    ids: &[i64],
    sep: &str,
    vars: &PasteVars,
    as_path: bool,
) -> Res<(Vec<Content>, usize)> {
    let db = state.db.lock().unwrap();
    let mut out = vec![];
    let mut clip = None;
    for id in ids {
        match db.content(*id).map_err(e)? {
            Some((k, _, Some(p))) if k == "image" => out.push(Content::Image(p.into())),
            Some((k, Some(t), _)) if k == "file" => {
                let paths: Vec<String> = t.lines().map(str::to_string).collect();
                if as_path {
                    out.push(Content::Text(paths.join("\n")));
                    continue;
                }
                let alive: Vec<String> =
                    paths.into_iter().filter(|p| std::path::Path::new(p).exists()).collect();
                if alive.is_empty() {
                    return Err("ファイルが見つかりません（移動または削除されています）".into());
                }
                // 続けて選んだファイルは 1 回でまとめて貼る
                if let Some(Content::Files(prev)) = out.last_mut() {
                    prev.extend(alive);
                } else {
                    out.push(Content::Files(alive));
                }
            }
            Some((_, Some(t), _)) => {
                let t = if db.is_saved(*id).map_err(e)? { expand_vars(&t, vars, &mut clip) } else { t };
                out.push(Content::Text(t));
            }
            _ => {}
        }
    }
    // 全部テキストなら区切り文字でつないで 1 回で貼る
    if out.len() > 1 && out.iter().all(|c| matches!(c, Content::Text(_))) {
        let joined = out
            .iter()
            .map(|c| match c {
                Content::Text(t) => t.as_str(),
                _ => "",
            })
            .collect::<Vec<_>>()
            .join(sep);
        out = vec![Content::Text(joined)];
    }
    // 最初の目印の位置をカーソル位置にして、目印は全部消す
    let mut back = 0;
    let single = out.len() == 1;
    for c in out.iter_mut() {
        if let Content::Text(t) = c {
            if let Some(pos) = t.find(CURSOR_MARK) {
                if single {
                    back = t[pos..].chars().filter(|&ch| ch != CURSOR_MARK).count();
                }
                *t = t.replace(CURSOR_MARK, "");
            }
        }
    }
    Ok((out, back))
}

// ───────────────────────── コマンド ─────────────────────────

#[tauri::command]
fn list_items(state: State<AppState>, filter: Filter) -> Res<Vec<Item>> {
    let mut items = state.db.lock().unwrap().list(&filter).map_err(e)?;
    // アイコンのファイルが消えていたら頭文字バッジ表示にする
    for it in items.iter_mut() {
        if it.source_icon.as_deref().is_some_and(|p| !std::path::Path::new(p).exists()) {
            it.source_icon = None;
        }
        if it.kind == "file" {
            let text = state.db.lock().unwrap().original_text(it.id).ok().flatten().unwrap_or_default();
            it.missing = !text.lines().all(|p| std::path::Path::new(p).exists());
        }
    }
    Ok(items)
}

#[tauri::command]
fn item_text(state: State<AppState>, id: i64) -> Res<Option<String>> {
    state.db.lock().unwrap().original_text(id).map_err(e)
}

#[tauri::command]
fn source_apps(state: State<AppState>) -> Res<Vec<SourceApp>> {
    state.db.lock().unwrap().source_apps().map_err(e)
}

#[tauri::command]
fn list_tags(state: State<AppState>) -> Res<Vec<Tag>> {
    state.db.lock().unwrap().list_tags().map_err(e)
}

#[tauri::command]
fn create_tag(state: State<AppState>, name: String, color: String) -> Res<Tag> {
    if name.trim().is_empty() {
        return Err("タグ名が空です".into());
    }
    state.db.lock().unwrap().create_tag(&name, &color).map_err(e)
}

#[tauri::command]
fn update_tag(app: AppHandle, state: State<AppState>, id: i64, name: String, color: String) -> Res<()> {
    if name.trim().is_empty() {
        return Err("タグ名が空です".into());
    }
    state
        .db
        .lock()
        .unwrap()
        .update_tag(id, &name, &color)
        .map_err(|err| match err {
            rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                format!("「{}」という名前のタグはすでにあります", name.trim())
            }
            other => other.to_string(),
        })?;
    let _ = app.emit("clips-changed", ());
    Ok(())
}

#[tauri::command]
fn delete_tag(app: AppHandle, state: State<AppState>, id: i64) -> Res<()> {
    state.db.lock().unwrap().delete_tag(id).map_err(e)?;
    let _ = app.emit("clips-changed", ());
    Ok(())
}

#[tauri::command]
fn toggle_tag(state: State<AppState>, item_id: i64, tag_id: i64) -> Res<bool> {
    state.db.lock().unwrap().toggle_tag(item_id, tag_id).map_err(e)
}

#[tauri::command]
fn set_tag(state: State<AppState>, item_ids: Vec<i64>, tag_id: i64, on: bool) -> Res<()> {
    let db = state.db.lock().unwrap();
    for id in item_ids {
        db.set_tag(id, tag_id, on).map_err(e)?;
    }
    Ok(())
}

#[tauri::command]
fn set_saved(state: State<AppState>, ids: Vec<i64>, saved: bool) -> Res<()> {
    let db = state.db.lock().unwrap();
    for id in ids {
        db.set_saved(id, saved).map_err(e)?;
    }
    Ok(())
}

#[tauri::command]
fn delete_items(state: State<AppState>, ids: Vec<i64>) -> Res<()> {
    let mut files = vec![];
    {
        let db = state.db.lock().unwrap();
        for id in ids {
            files.extend(db.delete(id).map_err(e)?);
        }
    }
    remove_files(files);
    Ok(())
}

#[tauri::command]
fn clear_history(state: State<AppState>) -> Res<()> {
    let files = state.db.lock().unwrap().clear_history().map_err(e)?;
    remove_files(files);
    Ok(())
}

/// 保存の追加・編集。text は画像なら None
#[tauri::command]
fn save_item(
    state: State<AppState>,
    id: Option<i64>,
    title: String,
    keyword: String,
    text: Option<String>,
) -> Res<i64> {
    if text.as_deref().is_some_and(|t| t.is_empty()) {
        return Err("本文が空です".into());
    }
    state
        .db
        .lock()
        .unwrap()
        .save_item(id, &title, &keyword, text.as_deref())
        .map_err(e)?
        .ok_or_else(|| "同じ内容がすでに保存されています".into())
}

/// 選択アイテムを貼り付ける。copy_only=true ならクリップボードに入れるだけ
#[tauri::command]
fn paste_items(
    app: AppHandle,
    state: State<AppState>,
    ids: Vec<i64>,
    copy_only: bool,
    vars: Option<PasteVars>,
    as_path: Option<bool>,
    transform: Option<String>,
) -> Res<()> {
    if ids.is_empty() {
        return Ok(());
    }
    state.queue.lock().unwrap().clear();
    let sep = state.settings.lock().unwrap().multi_separator.clone();
    let (mut contents, mut cursor_back) =
        load_contents(&state, &ids, &sep, &vars.unwrap_or_default(), as_path.unwrap_or(false))?;
    if let Some(t) = transform.as_deref() {
        for c in contents.iter_mut() {
            if let Content::Text(s) = c {
                *s = transform_text(s, t);
            }
        }
        cursor_back = 0;
    }
    hide_panel_inner(&app);

    let app2 = app.clone();
    std::thread::spawn(move || {
        let state = app2.state::<AppState>();
        state.pasting.store(true, Ordering::SeqCst);
        let result = (|| -> Res<()> {
            if copy_only {
                if let Some(c) = contents.first() {
                    write_clipboard(c)?;
                }
                platform::activate_target();
                return Ok(());
            }
            platform::activate_target();
            std::thread::sleep(Duration::from_millis(150));
            if !platform::accessibility_trusted() {
                // 自動貼り付けできないので、クリップボードに入れるだけにする
                if let Some(c) = contents.first() {
                    write_clipboard(c)?;
                }
                let _ = app2.emit("need-accessibility", ());
                return Ok(());
            }
            for c in &contents {
                write_clipboard(c)?;
                std::thread::sleep(Duration::from_millis(40));
                send_paste_keys()?;
                std::thread::sleep(Duration::from_millis(180));
            }
            if cursor_back > 0 {
                send_left_keys(cursor_back)?;
            }
            Ok(())
        })();
        if let Err(err) = result {
            log::warn!("paste failed: {err}");
        }
        std::thread::sleep(Duration::from_millis(300));
        state.last_count.store(platform::change_count(), Ordering::SeqCst);
        state.pasting.store(false, Ordering::SeqCst);
        {
            let db = state.db.lock().unwrap();
            for id in &ids {
                let _ = db.touch(*id);
            }
        }
        let _ = app2.emit("clips-changed", ());
    });
    Ok(())
}

/// 形を変えて貼る
fn transform_text(s: &str, how: &str) -> String {
    match how {
        // 全角の英数字・記号・空白 → 半角
        "half" => s
            .chars()
            .map(|c| match c {
                '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
                '\u{3000}' => ' ',
                _ => c,
            })
            .collect(),
        // 半角の英数字・記号・空白 → 全角
        "full" => s
            .chars()
            .map(|c| match c {
                '!'..='~' => char::from_u32(c as u32 + 0xFEE0).unwrap_or(c),
                ' ' => '\u{3000}',
                _ => c,
            })
            .collect(),
        // 改行を消す（前後の空白も詰める）
        "oneline" => s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(""),
        "trim" => s.lines().map(str::trim).collect::<Vec<_>>().join("\n").trim().to_string(),
        "upper" => s.to_uppercase(),
        "lower" => s.to_lowercase(),
        _ => s.to_string(),
    }
}

/// 「続けて貼る」を始める。1 件目をクリップボードに入れ、⌘V / Ctrl+V のたびに次を入れる
#[tauri::command]
fn start_queue(app: AppHandle, state: State<AppState>, ids: Vec<i64>, vars: Option<PasteVars>) -> Res<usize> {
    let vars = vars.unwrap_or_default();
    let mut items = std::collections::VecDeque::new();
    for id in &ids {
        // 1 件ずつ（つながない）
        let (mut cs, _) = load_contents(&state, &[*id], "", &vars, false)?;
        if let Some(c) = cs.pop() {
            items.push_back((*id, c));
        }
    }
    let n = items.len();
    let Some((first_id, first)) = items.pop_front() else { return Ok(0) };
    start_tap_watch(&app);
    hide_panel_inner(&app);
    state.pasting.store(true, Ordering::SeqCst);
    let r = write_clipboard(&first);
    platform::activate_target();
    std::thread::sleep(Duration::from_millis(200));
    state.last_count.store(platform::change_count(), Ordering::SeqCst);
    state.pasting.store(false, Ordering::SeqCst);
    r?;
    let _ = state.db.lock().unwrap().touch(first_id);
    *state.queue.lock().unwrap() = items;
    let _ = app.emit("clips-changed", ());
    Ok(n)
}

/// ユーザーが ⌘V / Ctrl+V を押した: 「続けて貼る」中なら少し待って次をクリップボードへ
fn on_user_paste(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.queue.lock().unwrap().is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // 貼り付け先がいまのクリップボードを読み終えるのを待つ
        std::thread::sleep(Duration::from_millis(350));
        let state = app.state::<AppState>();
        let Some((id, next)) = state.queue.lock().unwrap().pop_front() else { return };
        state.pasting.store(true, Ordering::SeqCst);
        if let Err(err) = write_clipboard(&next) {
            log::warn!("queue paste failed: {err}");
        }
        std::thread::sleep(Duration::from_millis(150));
        state.last_count.store(platform::change_count(), Ordering::SeqCst);
        state.pasting.store(false, Ordering::SeqCst);
        let _ = state.db.lock().unwrap().touch(id);
        let _ = app.emit("clips-changed", ());
    });
}

/// リンクをブラウザで開く（http/https だけ）
#[tauri::command]
fn open_url(url: String) -> Res<()> {
    let url = url.trim();
    let ok = (url.starts_with("https://") || url.starts_with("http://"))
        && !url.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'' || c.is_control());
    if !ok {
        return Err("開けないリンクです".into());
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    r.map(|_| ()).map_err(e)
}

#[tauri::command]
fn set_paused(app: AppHandle, state: State<AppState>, paused: bool) {
    state.paused.store(paused, Ordering::SeqCst);
    // 再開時、止めていたあいだのコピーは拾わない
    state.last_count.store(platform::change_count(), Ordering::SeqCst);
    update_tray_pause(&app, paused);
    let _ = app.emit("paused-changed", paused);
}

#[tauri::command]
fn get_paused(state: State<AppState>) -> bool {
    state.paused.load(Ordering::SeqCst)
}

/// 保存したテキストをファイルに書き出す（ダウンロードフォルダ）。保存先のパスを返す
#[tauri::command]
fn export_saved(app: AppHandle, state: State<AppState>) -> Res<String> {
    let data = state.db.lock().unwrap().export_saved().map_err(e)?;
    let dir = app.path().download_dir().or_else(|_| app.path().home_dir()).map_err(e)?;
    let name = format!("Clipo-保存-{}.json", chrono_like_date());
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_string_pretty(&data).map_err(e)?).map_err(e)?;
    Ok(path.to_string_lossy().to_string())
}

/// 書き出したファイルの中身を読み込む。(追加, 更新) の件数
#[tauri::command]
fn import_saved(app: AppHandle, state: State<AppState>, json: String) -> Res<(usize, usize)> {
    let data: ExportFile = serde_json::from_str(&json).map_err(|_| "Clipo の書き出しファイルではありません".to_string())?;
    if data.app != "Clipo" {
        return Err("Clipo の書き出しファイルではありません".into());
    }
    let r = state.db.lock().unwrap().import_saved(&data).map_err(e)?;
    let _ = app.emit("clips-changed", ());
    Ok(r)
}

/// ファイル名用の日付（YYYYMMDD-HHMMSS、UTC）
fn chrono_like_date() -> String {
    let secs = now_ms() / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // 1970-01-01 からの日数 → 年月日（Howard Hinnant のアルゴリズム）
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// テキストを翻訳してカードに保存し、訳文を返す
#[tauri::command]
async fn translate_item(app: AppHandle, id: i64) -> Res<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let text = state
            .db
            .lock()
            .unwrap()
            .original_text(id)
            .map_err(e)?
            .ok_or("テキストがありません")?;
        let s = state.settings.lock().unwrap().clone();
        let target = translate::pick_target(&text, &s.translate_target);
        let out = translate::translate(&text, &s.translate_provider, &s.deepl_api_key, &target)?;
        state.db.lock().unwrap().set_translation(id, Some(&out)).map_err(e)?;
        Ok(out)
    })
    .await
    .map_err(e)?
}

#[tauri::command]
fn revert_translation(state: State<AppState>, id: i64) -> Res<()> {
    state.db.lock().unwrap().set_translation(id, None).map_err(e)
}

/// 履歴の画像を文字起こしして、新しいテキストカードとして追加
#[tauri::command]
async fn ocr_item(app: AppHandle, id: i64) -> Res<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let path = match state.db.lock().unwrap().content(id).map_err(e)? {
            Some((_, _, Some(p))) => PathBuf::from(p),
            _ => return Err("画像がありません".to_string()),
        };
        let text = platform::ocr_image_file(&path)?;
        if text.trim().is_empty() {
            return Err("文字が見つかりませんでした".into());
        }
        let hash = text_hash(&text);
        finish_insert(
            &app,
            &state,
            &NewItem {
                kind: "text",
                text: Some(&text),
                image_path: None,
                thumb_path: None,
                width: None,
                height: None,
                hash: &hash,
                source_name: Some("文字起こし"),
                source_id: Some("clipo.ocr"),
                source_icon: None,
            },
        )
    })
    .await
    .map_err(e)?
}

#[tauri::command]
fn ocr_capture(app: AppHandle) {
    start_ocr(&app);
}

#[tauri::command]
fn hide_panel(app: AppHandle) {
    hide_panel_inner(&app);
    platform::activate_target();
}

// Windows ではウィンドウ生成を同期コマンド内で行うと WebView2 がデッドロックし
// 真っ白なまま固まるため、async コマンドにする (tauri-apps/wry#583)
#[tauri::command]
async fn open_settings(app: AppHandle) {
    open_settings_window(&app);
}

#[tauri::command]
fn take_screenshot(app: AppHandle) {
    start_screenshot(&app);
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

/// 保存して、登録できなかったショートカットのエラー一覧を返す
#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Res<Vec<String>> {
    let s = settings.normalized();
    s.save(&state.settings_path).map_err(e)?;
    *state.settings.lock().unwrap() = s.clone();
    let al = app.autolaunch();
    let _ = if s.launch_at_login { al.enable() } else { al.disable() };
    let files = state.db.lock().unwrap().prune(s.max_history).map_err(e)?;
    remove_files(files);
    let errors = register_shortcuts(&app, &s);
    if let Some(panel) = app.get_webview_window(PANEL) {
        platform::style_panel(&panel, s.panel_blur);
    }
    let _ = app.emit("settings-changed", ());
    Ok(errors)
}

/// 設定画面でキー入力を記録している間、既存ショートカットを止める
#[tauri::command]
fn pause_shortcuts(app: AppHandle, state: State<AppState>, paused: bool) {
    if paused {
        let _ = app.global_shortcut().unregister_all();
        state.double_taps.lock().unwrap().clear();
    } else {
        let s = state.settings.lock().unwrap().clone();
        register_shortcuts(&app, &s);
    }
}

#[tauri::command]
fn accessibility_status() -> bool {
    platform::accessibility_trusted()
}

#[tauri::command]
fn open_accessibility() {
    platform::open_accessibility_settings();
}

#[tauri::command]
fn platform_name() -> &'static str {
    std::env::consts::OS
}

// ───────────────────────── 起動 ─────────────────────────

fn init_state(app: &AppHandle) -> Res<AppState> {
    let data_dir = app.path().app_data_dir().map_err(e)?;
    std::fs::create_dir_all(data_dir.join("icons")).map_err(e)?;
    let settings_path = app.path().app_config_dir().map_err(e)?.join("settings.json");
    let settings = Settings::load(&settings_path).normalized();
    let db = Db::open(&data_dir.join("clipo.sqlite3")).map_err(e)?;
    Ok(AppState {
        db: Mutex::new(db),
        settings: Mutex::new(settings),
        settings_path,
        data_dir,
        pasting: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        queue: Mutex::new(Default::default()),
        last_count: AtomicI64::new(0),
        screenshot_until: AtomicI64::new(0),
        panel_height_now: std::sync::atomic::AtomicU64::new(PANEL_HEIGHT.to_bits()),
        shortcuts: Mutex::new(HashMap::new()),
        double_taps: Mutex::new(HashMap::new()),
    })
}

/// トレイの「記録を一時停止」項目（文言を切り替える）
struct TrayPause(MenuItem<tauri::Wry>);

fn update_tray_pause(app: &AppHandle, paused: bool) {
    if let Some(t) = app.try_state::<TrayPause>() {
        let _ = t.0.set_text(if paused { "記録を再開" } else { "記録を一時停止" });
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(if paused { "Clipo（記録を一時停止中）" } else { "Clipo" }));
    }
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "履歴を開く", true, None::<&str>)?;
    let shot = MenuItem::with_id(app, "shot", "範囲スクショ", true, None::<&str>)?;
    let ocr = MenuItem::with_id(app, "ocr", "範囲を文字起こし", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "設定…", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "記録を一時停止", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Clipo を終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &ocr, &shot, &pause, &settings, &quit])?;
    app.manage(TrayPause(pause.clone()));
    // macOS はメニューバーに合わせて白黒反転するテンプレート画像、Windows はカラー
    #[cfg(target_os = "macos")]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-color.png"))?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Clipo")
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_tray_icon_event(|tray, ev| {
            // Windows/Linux: 左クリックでパネル、右クリックでメニュー
            #[cfg(not(target_os = "macos"))]
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = ev
            {
                show_panel(tray.app_handle());
            }
            #[cfg(target_os = "macos")]
            let _ = (tray, ev);
        })
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "open" => show_panel(app),
            "shot" => start_screenshot(app),
            "pause" => {
                let state = app.state::<AppState>();
                let paused = !state.paused.load(Ordering::SeqCst);
                set_paused(app.clone(), state, paused);
            }
            "ocr" => start_ocr(app),
            // メニューイベントハンドラ内での生成も Windows でデッドロックするので別スレッドで行う
            "settings" => {
                let app = app.clone();
                std::thread::spawn(move || open_settings_window(&app));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let action = app.state::<AppState>().shortcuts.lock().unwrap().get(&shortcut.id()).copied();
                    if let Some(action) = action {
                        run_action(app, action);
                    }
                })
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let state = init_state(&handle)?;
            let settings = state.settings.lock().unwrap().clone();
            app.manage(state);

            for err in register_shortcuts(&handle, &settings) {
                log::warn!("{err}");
            }
            setup_tray(&handle)?;
            start_monitor(handle.clone());
            start_screenshot_watcher(handle.clone());

            if let Some(panel) = app.get_webview_window(PANEL) {
                let blur = handle.state::<AppState>().settings.lock().unwrap().panel_blur;
                platform::style_panel(&panel, blur);
                let h = handle.clone();
                panel.on_window_event(move |ev| {
                    if let tauri::WindowEvent::Focused(false) = ev {
                        hide_panel_inner(&h);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_items,
            item_text,
            source_apps,
            list_tags,
            create_tag,
            delete_tag,
            update_tag,
            toggle_tag,
            set_tag,
            set_saved,
            delete_items,
            clear_history,
            save_item,
            paste_items,
            hide_panel,
            resize_panel,
            open_settings,
            take_screenshot,
            translate_item,
            revert_translation,
            ocr_capture,
            ocr_item,
            get_settings,
            save_settings,
            pause_shortcuts,
            accessibility_status,
            open_accessibility,
            platform_name,
            start_queue,
            open_url,
            set_paused,
            get_paused,
            export_saved,
            import_saved,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Clipo")
        .run(|_app, event| {
            // ウィンドウを全部閉じても常駐し続ける
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms() {
        assert_eq!(transform_text("ＡＢＣ１２３！　ｘ", "half"), "ABC123! x");
        assert_eq!(transform_text("Ab1 !", "full"), "Ａｂ１　！");
        assert_eq!(transform_text("  a\n b \n\n c ", "oneline"), "abc");
        assert_eq!(transform_text("  a \n b  ", "trim"), "a\nb");
        assert_eq!(transform_text("aBc", "upper"), "ABC");
        assert_eq!(transform_text("日本語ＡＢ", "half"), "日本語AB");
    }

    #[test]
    fn date_format() {
        let s = chrono_like_date();
        assert_eq!(s.len(), 15);
        assert!(s.starts_with("20"));
    }

    #[test]
    fn cursor_mark() {
        let vars = PasteVars { date: "2026/11/11".into(), time: "10:00".into() };
        let mut clip = Some(None);
        let out = expand_vars("a{カーソル}bc {日付}", &vars, &mut clip);
        assert_eq!(out, format!("a{CURSOR_MARK}bc 2026/11/11"));
    }
}
