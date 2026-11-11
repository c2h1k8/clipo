//! 修飾キーの 2 回押し（例: Ctrl を素早く 2 回）を検出する。
//! OS ごとのキー監視（platform::start_modifier_watch）から down / up / other が呼ばれる。
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 設定文字列での接頭辞（例: "DoubleTap+Control"）
pub const PREFIX: &str = "DoubleTap+";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mod {
    Control,
    Shift,
    Alt,
    Command,
}

impl Mod {
    pub fn parse(acc: &str) -> Option<Mod> {
        match acc.strip_prefix(PREFIX)? {
            "Control" | "Ctrl" => Some(Mod::Control),
            "Shift" => Some(Mod::Shift),
            "Alt" | "Option" => Some(Mod::Alt),
            "Command" | "Cmd" | "Super" | "Meta" => Some(Mod::Command),
            _ => None,
        }
    }
}

/// 1 回の押下をタップとみなす最長時間
const MAX_PRESS: Duration = Duration::from_millis(350);
/// 1 回目のタップから 2 回目のタップを離すまでの最長時間
const MAX_GAP: Duration = Duration::from_millis(500);

#[derive(Default)]
struct State {
    /// いま押されている修飾キーと押した時刻
    down: Option<(Mod, Instant)>,
    /// 押してから他のキーが混ざっていない
    clean: bool,
    /// 直前のタップ
    last_tap: Option<(Mod, Instant)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static HANDLER: OnceLock<Box<dyn Fn(Mod) + Send + Sync>> = OnceLock::new();

/// 2 回押しを検出したときに呼ぶ処理を登録して、キー監視を始める（2 回目以降は何もしない）
pub fn start(handler: impl Fn(Mod) + Send + Sync + 'static) -> bool {
    if HANDLER.set(Box::new(handler)).is_err() {
        return false;
    }
    *STATE.lock().unwrap() = Some(State::default());
    true
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.lock().ok()?.as_mut().map(f)
}

pub fn down(m: Mod) {
    with(|s| {
        match s.down {
            Some((cur, _)) if cur == m => {} // キーリピート
            Some(_) => {
                // 別の修飾キーと同時押し → 組み合わせなのでタップではない
                s.clean = false;
                s.last_tap = None;
            }
            None => {
                s.down = Some((m, Instant::now()));
                s.clean = true;
            }
        }
    });
}

pub fn up(m: Mod) {
    let fired = with(|s| {
        let Some((cur, at)) = s.down else { return false };
        if cur != m {
            return false;
        }
        s.down = None;
        if !s.clean || at.elapsed() > MAX_PRESS {
            s.last_tap = None;
            return false;
        }
        match s.last_tap {
            Some((prev, t)) if prev == m && t.elapsed() <= MAX_GAP => {
                s.last_tap = None;
                true
            }
            _ => {
                s.last_tap = Some((m, Instant::now()));
                false
            }
        }
    })
    .unwrap_or(false);
    if fired {
        if let Some(h) = HANDLER.get() {
            h(m);
        }
    }
}

/// 修飾キー以外のキーが押された
pub fn other() {
    with(|s| {
        s.clean = false;
        s.last_tap = None;
    });
}
