//! Reads Right Alt straight from the keyboards (the user must be in the `input` group), so the
//! shortcut works the same under any compositor.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use evdev::{Device, EventSummary, KeyCode};

use crate::control::Command;

const PRESS: i32 = 1;
const RELEASE: i32 = 0;
const RESCAN: Duration = Duration::from_secs(3);

/// Turns key events into commands. Right Alt tapped alone toggles dictation with cleanup,
/// Shift + Right Alt toggles it without cleanup, and Esc cancels. Right Alt used together with
/// another key (an AltGr character or a shortcut) does nothing.
#[derive(Debug, Default)]
pub struct ChordTracker {
    alt_down: bool,
    other_key: bool,
    shift_down: bool,
    raw: bool,
}

impl ChordTracker {
    pub fn key(&mut self, code: KeyCode, value: i32) -> Option<Command> {
        match code {
            KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT => {
                self.shift_down = value != RELEASE;
                self.raw |= self.alt_down && self.shift_down;
                None
            }
            KeyCode::KEY_RIGHTALT => self.right_alt(value),
            KeyCode::KEY_ESC if value == PRESS => Some(Command::Cancel),
            _ => {
                self.other_key |= self.alt_down && value == PRESS;
                None
            }
        }
    }

    fn right_alt(&mut self, value: i32) -> Option<Command> {
        match value {
            PRESS => {
                self.alt_down = true;
                self.other_key = false;
                self.raw = self.shift_down;
                None
            }
            RELEASE => {
                let tapped = self.alt_down && !self.other_key;
                self.alt_down = false;
                tapped.then_some(Command::Toggle { raw: self.raw })
            }
            _ => None,
        }
    }
}

/// Watches every keyboard that has a Right Alt key, including ones plugged in later.
pub fn spawn(commands: Sender<Command>) {
    let watched = Arc::new(Mutex::new(HashSet::new()));
    thread::spawn(move || {
        let mut skipped = HashSet::new();
        loop {
            let paths = event_devices();
            // A path that disappeared can come back as a different device.
            skipped.retain(|path| paths.contains(path));
            for path in paths {
                if skipped.contains(&path) || watched.lock().unwrap().contains(&path) {
                    continue;
                }
                match Device::open(&path) {
                    Ok(device) if has_right_alt(&device) => {
                        watched.lock().unwrap().insert(path.clone());
                        watch(device, path, commands.clone(), Arc::clone(&watched));
                    }
                    Ok(_) => {
                        skipped.insert(path);
                    }
                    Err(err) => {
                        eprintln!("pickscribe: cannot read {}: {err}", path.display());
                        skipped.insert(path);
                    }
                }
            }
            thread::sleep(RESCAN);
        }
    });
}

fn event_devices() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("event"))
        })
        .collect()
}

fn has_right_alt(device: &Device) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::KEY_RIGHTALT))
}

fn watch(
    mut device: Device,
    path: PathBuf,
    commands: Sender<Command>,
    watched: Arc<Mutex<HashSet<PathBuf>>>,
) {
    thread::spawn(move || {
        let mut tracker = ChordTracker::default();
        while let Ok(events) = device.fetch_events() {
            for event in events {
                if let EventSummary::Key(_, code, value) = event.destructure()
                    && let Some(command) = tracker.key(code, value)
                    && commands.send(command).is_err()
                {
                    return;
                }
            }
        }
        // The keyboard went away; forget it so a replug is picked up by the next scan.
        watched.lock().unwrap().remove(&path);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(events: &[(KeyCode, i32)]) -> Vec<Command> {
        let mut tracker = ChordTracker::default();
        events
            .iter()
            .filter_map(|&(code, value)| tracker.key(code, value))
            .collect()
    }

    #[test]
    fn right_alt_tap_toggles_with_cleanup() {
        let commands = run(&[
            (KeyCode::KEY_RIGHTALT, PRESS),
            (KeyCode::KEY_RIGHTALT, 2),
            (KeyCode::KEY_RIGHTALT, RELEASE),
        ]);
        assert_eq!(commands, [Command::Toggle { raw: false }]);
    }

    #[test]
    fn shift_before_or_during_right_alt_toggles_raw() {
        let before = run(&[
            (KeyCode::KEY_LEFTSHIFT, PRESS),
            (KeyCode::KEY_RIGHTALT, PRESS),
            (KeyCode::KEY_RIGHTALT, RELEASE),
            (KeyCode::KEY_LEFTSHIFT, RELEASE),
        ]);
        let during = run(&[
            (KeyCode::KEY_RIGHTALT, PRESS),
            (KeyCode::KEY_RIGHTSHIFT, PRESS),
            (KeyCode::KEY_RIGHTSHIFT, RELEASE),
            (KeyCode::KEY_RIGHTALT, RELEASE),
        ]);
        assert_eq!(before, [Command::Toggle { raw: true }]);
        assert_eq!(during, [Command::Toggle { raw: true }]);
    }

    #[test]
    fn right_alt_combined_with_another_key_does_nothing() {
        let commands = run(&[
            (KeyCode::KEY_RIGHTALT, PRESS),
            (KeyCode::KEY_C, PRESS),
            (KeyCode::KEY_C, RELEASE),
            (KeyCode::KEY_RIGHTALT, RELEASE),
        ]);
        assert!(commands.is_empty());
    }

    #[test]
    fn a_new_tap_forgets_the_previous_combination() {
        let commands = run(&[
            (KeyCode::KEY_RIGHTALT, PRESS),
            (KeyCode::KEY_C, PRESS),
            (KeyCode::KEY_RIGHTALT, RELEASE),
            (KeyCode::KEY_RIGHTALT, PRESS),
            (KeyCode::KEY_RIGHTALT, RELEASE),
        ]);
        assert_eq!(commands, [Command::Toggle { raw: false }]);
    }

    #[test]
    fn escape_cancels() {
        assert_eq!(
            run(&[(KeyCode::KEY_ESC, PRESS), (KeyCode::KEY_ESC, RELEASE)]),
            [Command::Cancel]
        );
    }
}
