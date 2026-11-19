//! アプリ内アップデート（GitHub Releases の latest.json を見る）。
//!
//! ブラウザで dmg を落とし直すと macOS の Gatekeeper に毎回止められるが、
//! アプリ自身がダウンロードして置き換えれば止められない。署名（自己署名の証明書）も同じなので、
//! アクセシビリティ・画面収録の許可もそのまま残る。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::menu::MenuItem;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

/// 起動してから最初に確認するまでの時間（起動直後の処理と重ならないように）
const FIRST_CHECK: Duration = Duration::from_secs(30);
/// 常駐中に確認する間隔
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

const CHECK_TEXT: &str = "アップデートを確認";

static INSTALLING: AtomicBool = AtomicBool::new(false);

/// トレイの「アップデートを確認」項目（見つかったら文言を変える）
pub struct TrayUpdate(pub MenuItem<tauri::Wry>);

pub fn new_tray_item(app: &AppHandle) -> tauri::Result<MenuItem<tauri::Wry>> {
    MenuItem::with_id(app, "update", CHECK_TEXT, true, None::<&str>)
}

fn set_tray_text(app: &AppHandle, text: &str) {
    if let Some(t) = app.try_state::<TrayUpdate>() {
        let _ = t.0.set_text(text);
    }
}

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    current: String,
    /// 新しい版があればその版
    latest: Option<String>,
}

async fn check(app: &AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let latest = update.map(|u| u.version);
    match &latest {
        Some(v) => set_tray_text(app, &format!("v{v} にアップデートして再起動")),
        None => set_tray_text(app, CHECK_TEXT),
    }
    Ok(UpdateInfo { current, latest })
}

async fn install(app: &AppHandle) -> Result<(), String> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("アップデート中です".into());
    }
    let result = async {
        let update = app
            .updater()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or("最新版です")?;
        set_tray_text(app, &format!("v{} をダウンロード中…", update.version));
        // Windows はインストーラーを起動してここで終了する（インストーラーが Clipo を起動し直す）
        update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    }
    .await;
    match result {
        Ok(()) => app.restart(),
        Err(e) => {
            INSTALLING.store(false, Ordering::SeqCst);
            set_tray_text(app, CHECK_TEXT);
            Err(e)
        }
    }
}

/// トレイの項目を押したとき: 新しい版を見つけていれば入れる。まだなら確認だけする
pub fn on_tray_click(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let found = app
            .try_state::<TrayUpdate>()
            .and_then(|t| t.0.text().ok())
            .is_some_and(|t| t != CHECK_TEXT);
        if found {
            if let Err(e) = install(&app).await {
                log::warn!("update failed: {e}");
            }
            return;
        }
        set_tray_text(&app, "確認中…");
        match check(&app).await {
            Ok(UpdateInfo { latest: None, current }) => {
                set_tray_text(&app, &format!("最新版です（v{current}）"));
                tokio_sleep(Duration::from_secs(5)).await;
                set_tray_text(&app, CHECK_TEXT);
            }
            Ok(_) => {}
            Err(e) => {
                log::warn!("update check failed: {e}");
                set_tray_text(&app, "確認できませんでした");
                tokio_sleep(Duration::from_secs(5)).await;
                set_tray_text(&app, CHECK_TEXT);
            }
        }
    });
}

async fn tokio_sleep(d: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}

/// 常駐中に定期的に確認する（開発版はしない: 開発版が配布版に置き換わってしまうため）
pub fn start_auto_check(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio_sleep(FIRST_CHECK).await;
        loop {
            if !INSTALLING.load(Ordering::SeqCst) {
                if let Err(e) = check(&app).await {
                    log::warn!("update check failed: {e}");
                }
            }
            tokio_sleep(CHECK_EVERY).await;
        }
    });
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    check(&app).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    install(&app).await
}
