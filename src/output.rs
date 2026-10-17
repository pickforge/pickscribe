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

pub fn deliver(text: &str) -> Result<()> {
    copy(text)?;
    thread::sleep(PASTE_DELAY);
    let args = paste_args(active_window_class().as_deref());
    run("wtype", &args)
}

fn copy(text: &str) -> Result<()> {
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
fn active_window_class() -> Option<String> {
    let output = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .ok()?;
    let window: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    window["class"].as_str().map(str::to_string)
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
