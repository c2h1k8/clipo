use super::AppInfo;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::DataExchange::{
    GetClipboardOwner, GetClipboardSequenceNumber, IsClipboardFormatAvailable, RegisterClipboardFormatW,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow};

static TARGET_HWND: AtomicUsize = AtomicUsize::new(0);

pub fn change_count() -> i64 {
    unsafe { GetClipboardSequenceNumber() as i64 }
}

/// パスワード管理アプリなどが「履歴に残さないで」と指定したコピーか
pub fn is_concealed() -> bool {
    unsafe {
        [
            RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")),
            RegisterClipboardFormatW(w!("Clipboard Viewer Ignore")),
        ]
        .into_iter()
        .any(|fmt| fmt != 0 && IsClipboardFormatAvailable(fmt).is_ok())
    }
}

fn friendly_name(stem: &str) -> String {
    match stem.to_ascii_lowercase().as_str() {
        "chrome" => "Chrome".into(),
        "msedge" => "Edge".into(),
        "firefox" => "Firefox".into(),
        "explorer" => "エクスプローラー".into(),
        "screenclippinghost" | "snippingtool" => "切り取りツール".into(),
        "screenpresso" => "Screenpresso".into(),
        "code" => "VS Code".into(),
        "winword" => "Word".into(),
        "excel" => "Excel".into(),
        "powerpnt" => "PowerPoint".into(),
        "notepad" => "メモ帳".into(),
        "slack" => "Slack".into(),
        _ => {
            let mut c = stem.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => stem.into(),
            }
        }
    }
}

fn app_from_hwnd(hwnd: HWND) -> Option<AppInfo> {
    if hwnd.0.is_null() {
        return None;
    }
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let res = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(handle);
        res.ok()?;
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        let stem = Path::new(&path).file_stem()?.to_string_lossy().to_string();
        Some(AppInfo {
            id: stem.to_ascii_lowercase(),
            name: friendly_name(&stem),
            path: Some(path),
            pid,
        })
    }
}

/// クリップボードの持ち主 → 取れなければ前面ウィンドウ
pub fn source_app() -> Option<AppInfo> {
    unsafe {
        if let Ok(owner) = GetClipboardOwner() {
            if let Some(app) = app_from_hwnd(owner) {
                return Some(app);
            }
        }
        app_from_hwnd(GetForegroundWindow())
    }
}

pub fn screenshot_ui_running() -> bool {
    false
}

pub fn remember_target() {
    unsafe {
        let hwnd = GetForegroundWindow();
        if let Some(app) = app_from_hwnd(hwnd) {
            if app.pid != std::process::id() {
                TARGET_HWND.store(hwnd.0 as usize, Ordering::SeqCst);
            }
        }
    }
}

pub fn activate_target() {
    let h = TARGET_HWND.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = SetForegroundWindow(HWND(h as *mut _));
        }
    }
}

pub fn accessibility_trusted() -> bool {
    true
}

pub fn open_accessibility_settings() {}

/// exe のアイコン（32px）を PNG に書き出す
pub fn save_app_icon(app: &AppInfo, out: &Path) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL};

    const SIZE: i32 = 32;
    let Some(path) = &app.path else { return false };
    unsafe {
        let mut info = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(
            &HSTRING::from(path.as_str()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if ok == 0 || info.hIcon.is_invalid() {
            return false;
        }
        let hdc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE,
                biHeight: -SIZE, // 上から下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(dib) = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(hdc);
            let _ = DestroyIcon(info.hIcon);
            return false;
        };
        let old = SelectObject(hdc, dib.into());
        let drawn = DrawIconEx(hdc, 0, 0, info.hIcon, SIZE, SIZE, 0, None, DI_NORMAL).is_ok();
        let len = (SIZE * SIZE * 4) as usize;
        let mut rgba = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
        SelectObject(hdc, old);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(hdc);
        let _ = DestroyIcon(info.hIcon);
        if !drawn {
            return false;
        }
        // BGRA → RGBA。アルファを持たない古いアイコンは不透明として扱う
        let has_alpha = rgba.chunks_exact(4).any(|p| p[3] != 0);
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
            if !has_alpha {
                p[3] = 255;
            }
        }
        image::RgbaImage::from_raw(SIZE as u32, SIZE as u32, rgba)
            .map(|img| img.save(out).is_ok())
            .unwrap_or(false)
    }
}

/// パネルのウィンドウを Windows 11 風にする（すりガラス＋角丸）
pub fn style_panel(win: &tauri::WebviewWindow) {
    use tauri::window::{Effect, EffectState, EffectsBuilder};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    let _ = win.set_effects(EffectsBuilder::new().effect(Effect::Acrylic).state(EffectState::Active).build());
    if let Ok(hwnd) = win.hwnd() {
        let pref = DWMWCP_ROUND;
        unsafe {
            // Windows 10 では未対応なので失敗しても無視
            let _ = DwmSetWindowAttribute(
                HWND(hwnd.0),
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &pref as *const _ as *const core::ffi::c_void,
                std::mem::size_of_val(&pref) as u32,
            );
        }
    }
}

/// Windows 標準の範囲選択（Win+Shift+S と同じ画面）を起動
pub fn run_screenshot() {
    // 起動は即座に戻る。撮影待ちの判定は呼び出し側のタイムアウトで行う
    let _ = std::process::Command::new("explorer.exe").arg("ms-screenclip:").status();
}

/// run_screenshot が戻ったあと、撮影結果を待つ時間(ms)
pub const SCREENSHOT_GRACE_MS: i64 = 60_000;

/// CJK 文字の間に入る余計な空白を取り除く（Windows OCR は日本語を 1 文字ずつ区切るため）
fn tidy_cjk(s: &str) -> String {
    fn cjk(c: char) -> bool {
        matches!(c as u32, 0x3000..=0x30FF | 0x3400..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, c) in chars.iter().enumerate() {
        if *c == ' ' && i > 0 && i + 1 < chars.len() && (cjk(chars[i - 1]) || cjk(chars[i + 1])) {
            continue;
        }
        out.push(*c);
    }
    out
}

/// 日本語の OCR エンジンを優先して作る（無ければユーザーの言語設定から）
fn ocr_engine() -> Result<windows::Media::Ocr::OcrEngine, String> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Media::Ocr::OcrEngine;
    if let Ok(ja) = Language::CreateLanguage(&HSTRING::from("ja")) {
        if OcrEngine::IsLanguageSupported(&ja).unwrap_or(false) {
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&ja) {
                return Ok(engine);
            }
        }
    }
    OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
        "OCR が使えません。Windows の「設定 → 時刻と言語 → 言語」で日本語を追加し、\
         「光学式文字認識」をインストールしてください"
            .to_string()
    })
}

fn ocr_rgba(w: u32, h: u32, rgba: &[u8]) -> Result<String, String> {
    use image::imageops::FilterType;
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    let e = |err: windows::core::Error| err.message().to_string();

    let mut img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or("画像を読めませんでした")?;
    // クリップボードの画像はアルファが 0（透明）のことがあり、そのままだと真っ黒扱いで何も読めない。
    // 透明部分は白として不透明にする
    let all_clear = img.pixels().all(|p| p[3] == 0);
    for p in img.pixels_mut() {
        if all_clear {
            p[3] = 255;
        } else if p[3] < 255 {
            let a = p[3] as u32;
            for c in 0..3 {
                p[c] = ((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
    }
    // Windows OCR は小さい文字が苦手なので、小さい画像は拡大してから読む
    let max_dim = OcrEngine::MaxImageDimension().unwrap_or(10_000).max(1);
    let longest = w.max(h).max(1);
    let scale = if longest < 1600 { (1600 / longest).clamp(1, 3) } else { 1 };
    let scale = scale.min(max_dim / longest).max(1);
    if scale > 1 {
        img = image::imageops::resize(&img, w * scale, h * scale, FilterType::CatmullRom);
    } else if longest > max_dim {
        let f = max_dim as f64 / longest as f64;
        img = image::imageops::resize(&img, (w as f64 * f) as u32, (h as f64 * f) as u32, FilterType::Triangle);
    }
    let (w, h) = img.dimensions();

    let mut bgra = img.into_raw();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let writer = DataWriter::new().map_err(e)?;
    writer.WriteBytes(&bgra).map_err(e)?;
    let buffer = writer.DetachBuffer().map_err(e)?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        w as i32,
        h as i32,
        BitmapAlphaMode::Premultiplied,
    )
    .map_err(e)?;
    let engine = ocr_engine()?;
    let result = engine.RecognizeAsync(&bitmap).map_err(e)?.join().map_err(e)?;
    let mut lines = vec![];
    for line in result.Lines().map_err(e)? {
        lines.push(tidy_cjk(&line.Text().map_err(e)?.to_string()));
    }
    Ok(lines.join("\n"))
}

/// 範囲選択して文字起こし。キャンセル/タイムアウトなら None
pub fn capture_and_ocr(_tmp: &Path) -> Result<Option<String>, String> {
    let before = change_count();
    run_screenshot();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(SCREENSHOT_GRACE_MS as u64);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if change_count() == before {
            continue;
        }
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        if let Ok(img) = cb.get_image() {
            return ocr_rgba(img.width as u32, img.height as u32, &img.bytes).map(Some);
        }
        return Ok(None);
    }
    Ok(None)
}

pub fn ocr_image_file(path: &Path) -> Result<String, String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    ocr_rgba(w, h, img.as_raw())
}

/// Windows の標準スクショはクリップボードに入るので、フォルダ監視はしない
pub fn screenshot_dir() -> Option<std::path::PathBuf> {
    None
}

pub fn screenshot_prefixes() -> Vec<String> {
    vec![]
}

/// 修飾キーの 2 回押し検出用に、キーボードの低レベルフックを専用スレッドで動かす
pub fn start_modifier_watch() {
    use crate::taps::{self, Mod};
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL,
        WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            if code >= 0 {
                let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
                // 貼り付けなどで合成されたキー入力は数えない
                if (k.flags.0 & LLKHF_INJECTED.0) == 0 {
                    let m = match k.vkCode {
                        0xA0 | 0xA1 | 0x10 => Some(Mod::Shift),
                        0xA2 | 0xA3 | 0x11 => Some(Mod::Control),
                        0xA4 | 0xA5 | 0x12 => Some(Mod::Alt),
                        0x5B | 0x5C => Some(Mod::Command),
                        _ => None,
                    };
                    let msg = wparam.0 as u32;
                    let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                    let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
                    match (m, is_down, is_up) {
                        (Some(m), true, _) => taps::down(m),
                        (Some(m), _, true) => taps::up(m),
                        (None, true, _) => taps::other(),
                        _ => {}
                    }
                }
            }
            CallNextHookEx(None, code, wparam, lparam)
        }
    }

    std::thread::spawn(|| unsafe {
        let Ok(_hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0) else {
            log::warn!("keyboard hook failed");
            return;
        };
        // フックはこのスレッドのメッセージループで呼ばれる
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    });
}
