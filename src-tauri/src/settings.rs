use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// パネル表示ショートカット (例: "CommandOrControl+Shift+V")
    pub panel_shortcut: String,
    /// Clipo 経由の範囲スクショ（画像）を有効にするか
    pub screenshot_enabled: bool,
    /// 範囲スクショのショートカット
    pub screenshot_shortcut: String,
    /// OS 標準スクショ（⌘⇧4 など）で保存された画像も履歴とクリップボードに入れる
    pub watch_screenshots: bool,
    /// 範囲を選んで文字起こし（OCR）のショートカット
    pub ocr_shortcut: String,
    /// 翻訳サービス: "google"（キー不要）| "deepl"
    pub translate_provider: String,
    pub deepl_api_key: String,
    /// 訳先の言語（"ja" のとき日本語の文は英語へ訳す）
    pub translate_target: String,
    /// 履歴の最大件数（保存したものは数えない）
    pub max_history: u32,
    /// パネルのカードの段数（1〜3）
    pub panel_rows: u32,
    /// カードを日付ごとに区切って表示する
    pub group_by_date: bool,
    /// パネル背景の不透明度（0〜100）
    pub panel_opacity: u32,
    /// 背景をすりガラス（OS のぼかし効果）にする。オフなら後ろがそのまま透ける
    pub panel_blur: bool,
    /// カードの大きさ: "small" | "medium" | "large"
    pub card_size: String,
    /// 記録しないアプリ（macOS: bundle id / Windows: exe 名）
    pub excluded_apps: Vec<String>,
    /// 複数選択して貼り付けるときの区切り文字
    pub multi_separator: String,
    /// ログイン時に起動
    pub launch_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Windows の Ctrl+Shift+V は多くのアプリで「書式なし貼り付け」なので避ける
            panel_shortcut: if cfg!(target_os = "macos") {
                "CommandOrControl+Shift+V".into()
            } else {
                "Control+Alt+V".into()
            },
            screenshot_enabled: cfg!(target_os = "macos"),
            screenshot_shortcut: "CommandOrControl+Shift+2".into(),
            watch_screenshots: cfg!(target_os = "macos"),
            ocr_shortcut: "CommandOrControl+Shift+1".into(),
            translate_provider: "google".into(),
            deepl_api_key: String::new(),
            translate_target: "ja".into(),
            max_history: 500,
            panel_rows: 1,
            panel_opacity: 30,
            panel_blur: true,
            group_by_date: true,
            card_size: "medium".into(),
            excluded_apps: vec![],
            multi_separator: "\n".into(),
            launch_at_login: false,
        }
    }
}

impl Settings {
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }

    pub fn normalized(mut self) -> Self {
        self.max_history = self.max_history.clamp(10, 10_000);
        self.panel_rows = self.panel_rows.clamp(1, 3);
        self.panel_opacity = self.panel_opacity.min(100);
        if !matches!(self.card_size.as_str(), "small" | "medium" | "large") {
            self.card_size = "medium".into();
        }
        self
    }
}
