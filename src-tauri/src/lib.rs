mod db;
mod platform;
mod settings;
mod translate;

use db::{now_ms, Db, Filter, Item, NewItem, SourceApp, Tag};
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
/// 1 段のときのパネルの高さ。段を増やすごとに ROW_HEIGHT ずつ高くなる
const PANEL_HEIGHT: f64 = 300.0;
const ROW_HEIGHT: f64 = 216.0;
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
    last_count: AtomicI64,
    /// この時刻(ms)までに来た画像はスクショ扱い
    screenshot_until: AtomicI64,
    /// いま表示しているパネルの高さ（論理px, f64 のビット列）
    panel_height_now: std::sync::atomic::AtomicU64,
    shortcuts: Mutex<HashMap<u32, Action>>,
}

type Res<T> = Result<T, String>;
fn e<E: ToString>(err: E) -> String {
    err.to_string()
}

fn fnv1a(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}{:x}", bytes.len())
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
            // コピー元はできるだけ早く取る（ユーザーがアプリを切り替える前に）
            let src = platform::source_app();
            let shot = now_ms() < state.screenshot_until.load(Ordering::SeqCst)
                || platform::screenshot_ui_running();
            if let Err(err) = capture(&app, &state, src, shot, &mut tried_icons) {
                log::warn!("capture failed: {err}");
            }
        }
    });
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
            let hash = format!("t:{}", fnv1a(text.as_bytes()));
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
    let logical_h = logical_h.max(PANEL_HEIGHT);
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
    let max_h = PANEL_HEIGHT + (max_rows - 1.0) * ROW_HEIGHT + DAY_HEADER_HEIGHT;
    let h = height.round().clamp(PANEL_HEIGHT, max_h);
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
            let Some(text) = text.filter(|t| !t.trim().is_empty()) else { return Ok(false) };
            write_clipboard(&Content::Text(text.clone()))?;
            let hash = format!("t:{}", fnv1a(text.as_bytes()));
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

enum Content {
    Text(String),
    Image(PathBuf),
}

fn write_clipboard(c: &Content) -> Res<()> {
    let mut cb = open_clipboard()?;
    match c {
        Content::Text(t) => cb.set_text(t.clone()).map_err(e),
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

fn load_contents(state: &AppState, ids: &[i64], sep: &str) -> Res<Vec<Content>> {
    let db = state.db.lock().unwrap();
    let mut out = vec![];
    for id in ids {
        match db.content(*id).map_err(e)? {
            Some((k, _, Some(p))) if k == "image" => out.push(Content::Image(p.into())),
            Some((_, Some(t), _)) => out.push(Content::Text(t)),
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
        return Ok(vec![Content::Text(joined)]);
    }
    Ok(out)
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
fn set_favorite(state: State<AppState>, id: i64, favorite: bool) -> Res<()> {
    state.db.lock().unwrap().set_favorite(id, favorite).map_err(e)
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

#[tauri::command]
fn save_snippet(state: State<AppState>, id: Option<i64>, title: String, text: String) -> Res<i64> {
    if text.is_empty() {
        return Err("本文が空です".into());
    }
    state.db.lock().unwrap().save_snippet(id, &title, &text).map_err(e)
}

/// 選択アイテムを貼り付ける。copy_only=true ならクリップボードに入れるだけ
#[tauri::command]
fn paste_items(app: AppHandle, state: State<AppState>, ids: Vec<i64>, copy_only: bool) -> Res<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let sep = state.settings.lock().unwrap().multi_separator.clone();
    let contents = load_contents(&state, &ids, &sep)?;
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
        let hash = format!("t:{}", fnv1a(text.as_bytes()));
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
    let _ = app.emit("settings-changed", ());
    Ok(errors)
}

/// 設定画面でキー入力を記録している間、既存ショートカットを止める
#[tauri::command]
fn pause_shortcuts(app: AppHandle, state: State<AppState>, paused: bool) {
    if paused {
        let _ = app.global_shortcut().unregister_all();
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
        last_count: AtomicI64::new(0),
        screenshot_until: AtomicI64::new(0),
        panel_height_now: std::sync::atomic::AtomicU64::new(PANEL_HEIGHT.to_bits()),
        shortcuts: Mutex::new(HashMap::new()),
    })
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "履歴を開く", true, None::<&str>)?;
    let shot = MenuItem::with_id(app, "shot", "範囲スクショ", true, None::<&str>)?;
    let ocr = MenuItem::with_id(app, "ocr", "範囲を文字起こし", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "設定…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Clipo を終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &ocr, &shot, &settings, &quit])?;
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
                    match action {
                        Some(Action::TogglePanel) => toggle_panel(app),
                        Some(Action::Screenshot) => start_screenshot(app),
                        Some(Action::Ocr) => start_ocr(app),
                        None => {}
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
                platform::style_panel(&panel);
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
            set_favorite,
            delete_items,
            clear_history,
            save_snippet,
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

