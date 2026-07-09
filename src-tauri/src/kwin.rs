//! KWin window-rule helper for the float capsule.
//!
//! On Wayland, GTK cannot set keep-above / no-focus / skip-taskbar, so on KDE
//! we install a KWin window rule that forces them for the float window
//! (matched by window class substring + exact title). Best-effort: silently
//! does nothing on other desktops or when the KDE config tools are missing.

use std::path::Path;
use std::process::Command;

use pickscribe::engine::find_command;

const RULE_GROUP: &str = "pickscribe-float-keep-above";
const FLOAT_TITLE: &str = "PickScribe Float";
const WM_CLASS: &str = "pickscribe";

fn group_has_key(contents: &str, group: &str, key: &str) -> bool {
    let header = format!("[{group}]");
    let mut in_group = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_group = trimmed == header;
            continue;
        }
        if in_group && trimmed.starts_with(key) {
            return true;
        }
    }
    false
}

pub fn ensure_float_rule() {
    if std::env::var("XDG_SESSION_TYPE").as_deref() != Ok("wayland") {
        return;
    }
    if std::env::var("GDK_BACKEND").as_deref() == Ok("x11") {
        return;
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !desktop.to_uppercase().contains("KDE") {
        return;
    }
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let rules_path = Path::new(&home).join(".config/kwinrulesrc");
    let existing_rules = std::fs::read_to_string(&rules_path).unwrap_or_default();
    let already_registered = existing_rules.contains(RULE_GROUP);
    // "positionrule" inside our group marks the current revision; rewrite older ones.
    if already_registered && group_has_key(&existing_rules, RULE_GROUP, "positionrule") {
        return;
    }

    let Some((write_tool_name, write_tool)) = ["kwriteconfig6", "kwriteconfig5"]
        .into_iter()
        .find_map(|t| find_command(t).map(|path| (t, path)))
    else {
        return;
    };
    let read_tool = if write_tool_name == "kwriteconfig6" {
        "kreadconfig6"
    } else {
        "kreadconfig5"
    };

    let write = |key: &str, value: &str| {
        let _ = Command::new(&write_tool)
            .args([
                "--file",
                "kwinrulesrc",
                "--group",
                RULE_GROUP,
                "--key",
                key,
                value,
            ])
            .status();
    };

    write("Description", "PickScribe float capsule (managed by PickScribe)");
    // Match: window class contains "pickscribe" AND title is exactly the
    // float window title, so the main window is unaffected.
    write("wmclass", WM_CLASS);
    write("wmclassmatch", "2"); // substring
    write("title", FLOAT_TITLE);
    write("titlematch", "1"); // exact
    // Force (rule value 2): keep above, never take focus, stay out of the
    // taskbar, pager, and window switcher.
    write("above", "true");
    write("aboverule", "2");
    write("acceptfocus", "false");
    write("acceptfocusrule", "2");
    write("skiptaskbar", "true");
    write("skiptaskbarrule", "2");
    write("skippager", "true");
    write("skippagerrule", "2");
    write("skipswitcher", "true");
    write("skipswitcherrule", "2");
    // Initial spot only (rule 3 = apply initially) — the capsule stays
    // draggable, and each Pickforge app gets its own row to avoid stacking.
    write("position", "64,64");
    write("positionrule", "3");

    // Register the rule group in the [General] rules list (Plasma 6 format).
    if !already_registered && let Some(read_tool) = find_command(read_tool) {
        let existing = Command::new(read_tool)
            .args(["--file", "kwinrulesrc", "--group", "General", "--key", "rules"])
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .unwrap_or_default();
        let rules = if existing.is_empty() {
            RULE_GROUP.to_string()
        } else {
            format!("{existing},{RULE_GROUP}")
        };
        let _ = Command::new(&write_tool)
            .args([
                "--file",
                "kwinrulesrc",
                "--group",
                "General",
                "--key",
                "rules",
                &rules,
            ])
            .status();
    }

    // Ask KWin to reload its rules.
    let _ = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.kde.KWin",
            "--object-path",
            "/KWin",
            "--method",
            "org.kde.KWin.reconfigure",
        ])
        .output();
}
