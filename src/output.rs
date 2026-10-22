//! Puts the text on the clipboard and sends the paste shortcut to the focused window. Pasting
//! keeps line breaks inside the field, where typed Enter keys would send a chat message early.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// Gives the clipboard owner time to settle before the paste shortcut arrives.
const PASTE_DELAY: Duration = Duration::from_millis(60);

const TERMINALS: &[&str] = &[
    "alacritty",
    "foot",
    "ghostty",
    "kitty",
    "konsole",
    "terminal",
    "wezterm",
    "xterm",
];

#[derive(Debug, PartialEq)]
pub enum Delivery {
    Pasted,
    FocusChanged,
    CopyFailed,
    PasteFailed,
    SessionLocked { copied: bool },
}

impl Delivery {
    pub fn notice(&self, saved: bool) -> Option<&'static str> {
        match self {
            Self::Pasted => None,
            Self::FocusChanged => Some("Focus changed · text copied"),
            Self::CopyFailed if saved => Some("Paste failed · text in history"),
            Self::CopyFailed => Some("Paste failed · text lost"),
            Self::PasteFailed => Some("Paste failed · text copied"),
            Self::SessionLocked { copied: true } => Some("Session locked · text copied"),
            Self::SessionLocked { copied: false } if saved => {
                Some("Session locked · text in history")
            }
            Self::SessionLocked { copied: false } => Some("Session locked · text lost"),
        }
    }
}

#[derive(Default)]
pub struct Focus {
    hyprland: bool,
    address: Option<String>,
}

impl Focus {
    pub fn capture() -> Self {
        let hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some();
        Self {
            hyprland,
            address: hyprland
                .then(active_window)
                .flatten()
                .and_then(|window| window["address"].as_str().map(str::to_string)),
        }
    }

    fn changed(&self, current: Option<&str>) -> bool {
        self.hyprland && self.address.as_deref() != current
    }
}

pub fn deliver(
    text: &str,
    focus: &Focus,
    latched: impl Fn() -> bool,
    locked: impl Fn() -> bool,
) -> Delivery {
    deliver_with(
        text,
        latched,
        |text| {
            copy(text)?;
            thread::sleep(PASTE_DELAY);
            Ok(())
        },
        || paste_with(focus, locked, active_window, |args| run("wtype", args)),
    )
}

fn deliver_with(
    text: &str,
    latched: impl Fn() -> bool,
    copy: impl FnOnce(&str) -> Result<()>,
    paste: impl FnOnce() -> Delivery,
) -> Delivery {
    if latched() {
        return Delivery::SessionLocked { copied: false };
    }
    if let Err(err) = copy(text) {
        eprintln!("pickscribe: copy failed: {err:#}");
        return Delivery::CopyFailed;
    }
    paste()
}

fn paste_with(
    focus: &Focus,
    locked: impl Fn() -> bool,
    window: impl FnOnce() -> Option<serde_json::Value>,
    dispatch: impl FnOnce(&[&str]) -> Result<()>,
) -> Delivery {
    let window = window();
    if focus.changed(
        window
            .as_ref()
            .and_then(|window| window["address"].as_str()),
    ) {
        return if locked() {
            Delivery::SessionLocked { copied: true }
        } else {
            Delivery::FocusChanged
        };
    }
    let args = paste_args(window.as_ref().and_then(|window| window["class"].as_str()));
    if locked() {
        return Delivery::SessionLocked { copied: true };
    }
    match dispatch(&args) {
        Ok(()) => Delivery::Pasted,
        Err(err) => {
            eprintln!("pickscribe: paste failed: {err:#}");
            Delivery::PasteFailed
        }
    }
}

pub fn copy(text: &str) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .context("wl-copy is not installed")?;
    child
        .stdin
        .take()
        .context("wl-copy stdin")?
        .write_all(text.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        bail!("wl-copy failed with {status}");
    }
    Ok(())
}

/// Terminals paste with Ctrl+Shift+V; everything else with Ctrl+V.
fn paste_args(class: Option<&str>) -> Vec<&'static str> {
    let terminal = class.is_some_and(|class| {
        let class = class.to_lowercase();
        TERMINALS.iter().any(|name| class.contains(name))
    });
    if terminal {
        vec![
            "-M", "ctrl", "-M", "shift", "-k", "v", "-m", "shift", "-m", "ctrl",
        ]
    } else {
        vec!["-M", "ctrl", "-k", "v", "-m", "ctrl"]
    }
}

/// Hyprland reports the focused window; other compositors fall back to Ctrl+V.
fn active_window() -> Option<serde_json::Value> {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let output = match Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            eprintln!("pickscribe: cannot read Hyprland focus: {err}");
            return None;
        }
    };
    if !output.status.success() {
        eprintln!(
            "pickscribe: hyprctl activewindow failed with {}",
            output.status
        );
        return None;
    }
    match serde_json::from_slice(&output.stdout) {
        Ok(window) => Some(window),
        Err(_) => {
            eprintln!("pickscribe: hyprctl activewindow returned invalid JSON");
            None
        }
    }
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("{program} is not installed"))?;
    if !status.success() {
        bail!("{program} failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_guard_decision_table() {
        for (before, after, changed) in [
            (Some("0x1"), Some("0x1"), false),
            (Some("0x1"), Some("0x2"), true),
            (Some("0x1"), None, true),
            (None, Some("0x1"), true),
            (None, None, false),
        ] {
            for hyprland in [true, false] {
                let focus = Focus {
                    hyprland,
                    address: before.map(str::to_string),
                };
                assert_eq!(focus.changed(after), hyprland && changed);
            }
        }
    }

    #[test]
    fn delivery_notices_distinguish_clipboard_failure() {
        assert_eq!(
            Delivery::CopyFailed.notice(true),
            Some("Paste failed · text in history")
        );
        assert_eq!(
            Delivery::CopyFailed.notice(false),
            Some("Paste failed · text lost")
        );
        assert_eq!(
            Delivery::PasteFailed.notice(false),
            Some("Paste failed · text copied")
        );
        assert_eq!(
            Delivery::FocusChanged.notice(false),
            Some("Focus changed · text copied")
        );
        assert_eq!(Delivery::Pasted.notice(false), None);
        assert_eq!(
            Delivery::SessionLocked { copied: false }.notice(true),
            Some("Session locked · text in history")
        );
        assert_eq!(
            Delivery::SessionLocked { copied: false }.notice(false),
            Some("Session locked · text lost")
        );
    }

    #[test]
    fn locked_delivery_never_touches_the_clipboard() {
        let focus = Focus {
            hyprland: false,
            address: None,
        };
        assert_eq!(
            deliver("unused", &focus, || true, || panic!("unexpected check")),
            Delivery::SessionLocked { copied: false }
        );
    }

    #[test]
    fn delivery_checks_after_focus_and_immediately_before_dispatch() {
        let steps = std::cell::RefCell::new(Vec::new());
        let focus = Focus {
            hyprland: true,
            address: Some("0x1".into()),
        };
        let delivery = deliver_with(
            "dictation",
            || {
                steps.borrow_mut().push("latch");
                false
            },
            |_| {
                steps.borrow_mut().push("copy");
                Ok(())
            },
            || {
                paste_with(
                    &focus,
                    || {
                        steps.borrow_mut().push("lock");
                        false
                    },
                    || {
                        steps.borrow_mut().push("focus");
                        Some(serde_json::json!({"address": "0x1", "class": "kitty"}))
                    },
                    |args| {
                        steps.borrow_mut().push("dispatch");
                        assert!(args.contains(&"shift"));
                        Ok(())
                    },
                )
            },
        );
        assert_eq!(delivery, Delivery::Pasted);
        assert_eq!(
            *steps.borrow(),
            ["latch", "copy", "focus", "lock", "dispatch"]
        );
    }

    #[test]
    fn lock_during_focus_query_prevents_dispatch_and_reports_the_clipboard() {
        let locked = std::cell::Cell::new(false);
        let copied = std::cell::Cell::new(false);
        let focus = Focus {
            hyprland: true,
            address: Some("0x1".into()),
        };
        let delivery = deliver_with(
            "dictation",
            || false,
            |_| {
                copied.set(true);
                Ok(())
            },
            || {
                paste_with(
                    &focus,
                    || locked.get(),
                    || {
                        locked.set(true);
                        Some(serde_json::json!({"address": "0x1"}))
                    },
                    |_| panic!("locked delivery must not dispatch"),
                )
            },
        );
        assert!(copied.get());
        assert_eq!(delivery, Delivery::SessionLocked { copied: true });
        for saved in [true, false] {
            assert_eq!(delivery.notice(saved), Some("Session locked · text copied"));
        }
    }

    #[test]
    fn lock_notice_takes_precedence_when_focus_also_changes() {
        let focus = Focus {
            hyprland: true,
            address: Some("0x1".into()),
        };
        let delivery = paste_with(
            &focus,
            || true,
            || Some(serde_json::json!({"address": "0x2"})),
            |_| panic!("changed focus must not dispatch"),
        );
        assert_eq!(delivery, Delivery::SessionLocked { copied: true });
    }

    #[test]
    fn terminals_get_ctrl_shift_v() {
        for class in [
            "Alacritty",
            "com.mitchellh.ghostty",
            "org.gnome.Terminal",
            "kitty",
        ] {
            assert!(paste_args(Some(class)).contains(&"shift"), "{class}");
        }
    }

    #[test]
    fn other_windows_and_unknown_focus_get_ctrl_v() {
        assert_eq!(
            paste_args(Some("zen")),
            ["-M", "ctrl", "-k", "v", "-m", "ctrl"]
        );
        assert_eq!(paste_args(None), ["-M", "ctrl", "-k", "v", "-m", "ctrl"]);
    }
}
