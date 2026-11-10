#[derive(Debug, Clone)]
pub struct AppInfo {
    /// macOS: bundle id / Windows: exe 名（小文字）
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub pid: u32,
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;
