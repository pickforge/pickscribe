use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::config::PasteConfig;

use super::{command_exists, find_command};

pub fn copy_to_clipboard(text: &str) -> Result<()> {
    if let Some(wl_copy) = find_command("wl-copy") {
        // wl-copy daemonizes; don't wait on stdout.
        let mut child = Command::new(wl_copy)
            .stdin(Stdio::piped())
            .spawn()
            .context("starting wl-copy")?;
        child
            .stdin
            .take()
            .context("wl-copy stdin")?
            .write_all(text.as_bytes())?;
        let _ = child.wait();
        return Ok(());
    }
    for (cmd, args) in [
        ("xclip", vec!["-selection", "clipboard"]),
        ("xsel", vec!["--clipboard", "--input"]),
    ] {
        if let Some(program) = find_command(cmd) {
            let mut child = Command::new(program)
                .args(&args)
                .stdin(Stdio::piped())
                .spawn()
                .with_context(|| format!("starting {cmd}"))?;
            child
                .stdin
                .take()
                .context("clipboard stdin")?
                .write_all(text.as_bytes())?;
            let status = child.wait()?;
            if !status.success() {
                bail!("{cmd} exited with {status}");
            }
            return Ok(());
        }
    }
    bail!("no clipboard tool found (wl-copy, xclip, xsel)")
}

/// Copy `text` to the clipboard and paste it into the focused window.
pub fn deliver(cfg: &PasteConfig, text: &str) -> Result<()> {
    if cfg.copy_to_clipboard || cfg.method != "type" {
        copy_to_clipboard(text)?;
    }
    match cfg.method.as_str() {
        "none" => Ok(()),
        "type" => {
            std::thread::sleep(Duration::from_millis(cfg.delay_ms));
            type_text(text)
        }
        _ => {
            // "auto" / "hotkey"
            std::thread::sleep(Duration::from_millis(cfg.delay_ms));
            if command_exists("ydotool") || command_exists("xdotool") {
                paste_with_hotkey(&cfg.chord)
            } else if cfg.method == "hotkey" {
                bail!("no paste tool found (ydotool, xdotool)")
            } else {
                type_text(text)
            }
        }
    }
}

fn paste_with_hotkey(chord: &str) -> Result<()> {
    let shift = chord == "ctrl-shift-v";
    if let Some(ydotool) = find_command("ydotool") {
        // Release every modifier the user may still be holding, then send the chord.
        let mut keys: Vec<&str> = vec![
            "29:0", "97:0", "42:0", "54:0", "56:0", "100:0", "125:0", "126:0",
        ];
        if shift {
            keys.extend(["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]);
        } else {
            keys.extend(["29:1", "47:1", "47:0", "29:0"]);
        }
        let status = Command::new(ydotool)
            .arg("key")
            .args(&keys)
            .status()
            .context("running ydotool")?;
        if !status.success() {
            bail!("ydotool key failed — is ydotool.service running?");
        }
        return Ok(());
    }
    if let Some(xdotool) = find_command("xdotool") {
        let combo = if shift { "ctrl+shift+v" } else { "ctrl+v" };
        let status = Command::new(xdotool)
            .args(["key", "--clearmodifiers", combo])
            .status()
            .context("running xdotool")?;
        if !status.success() {
            bail!("xdotool key failed");
        }
        return Ok(());
    }
    bail!("no paste tool found (ydotool, xdotool)")
}

fn type_text(text: &str) -> Result<()> {
    let wayland = std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false);
    let order: &[&str] = if wayland {
        &["ydotool", "wtype", "xdotool"]
    } else {
        &["xdotool", "ydotool", "wtype"]
    };
    for tool in order {
        let Some(program) = find_command(tool) else {
            continue;
        };
        return match *tool {
            "ydotool" => pipe_type(&program, tool, &["type", "--file", "-"], text),
            "xdotool" => pipe_type(&program, tool, &["type", "--clearmodifiers", "--file", "-"], text),
            "wtype" => {
                let status = Command::new(program).arg(text).status()?;
                if !status.success() {
                    bail!("wtype failed");
                }
                Ok(())
            }
            _ => unreachable!(),
        };
    }
    bail!("no typing tool found (ydotool, xdotool, wtype)")
}

fn pipe_type(program: &std::path::Path, name: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {name}"))?;
    child
        .stdin
        .take()
        .context("type stdin")?
        .write_all(text.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        bail!("{name} type failed");
    }
    Ok(())
}
