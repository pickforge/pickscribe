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
}

impl Delivery {
    pub fn notice(&self) -> Option<&'static str> {
        match self {
            Self::Pasted => None,
            Self::FocusChanged => Some("Focus changed · text copied"),
            Self::CopyFailed => Some("Paste failed · text in history"),
            Self::PasteFailed => Some("Paste failed · text copied"),
        }
    }
}

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
        self.hyprland && (self.address.is_none() || self.address.as_deref() != current)
    }
}

pub fn deliver(text: &str, focus: &Focus) -> Delivery {
    if let Err(err) = copy(text) {
        eprintln!("pickscribe: copy failed: {err:#}");
        return Delivery::CopyFailed;
    }
    thread::sleep(PASTE_DELAY);
    let window = active_window();
    if focus.changed(
        window
            .as_ref()
            .and_then(|window| window["address"].as_str()),
    ) {
        return Delivery::FocusChanged;
    }
    let args = paste_args(window.as_ref().and_then(|window| window["class"].as_str()));
    match run("wtype", &args) {
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
    let output = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
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
    fn focus_changes_and_unreadable_hyprland_focus_skip_pasting() {
        let focus = Focus {
            hyprland: true,
            address: Some("0x1".into()),
        };
        assert!(!focus.changed(Some("0x1")));
        assert!(focus.changed(Some("0x2")));
        assert!(focus.changed(None));
        assert!(
            Focus {
                hyprland: true,
                address: None
            }
            .changed(None)
        );
        assert!(
            !Focus {
                hyprland: false,
                address: None
            }
            .changed(None)
        );
    }

    #[test]
    fn delivery_notices_distinguish_clipboard_failure() {
        assert_eq!(
            Delivery::CopyFailed.notice(),
            Some("Paste failed · text in history")
        );
        assert_eq!(
            Delivery::PasteFailed.notice(),
            Some("Paste failed · text copied")
        );
        assert_eq!(
            Delivery::FocusChanged.notice(),
            Some("Focus changed · text copied")
        );
        assert_eq!(Delivery::Pasted.notice(), None);
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
