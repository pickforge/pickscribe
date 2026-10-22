//! Reads Right Alt straight from the keyboards (the user must be in the `input` group), so the
//! shortcut works the same under any compositor.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
    held: HashSet<(PathBuf, KeyCode)>,
    alt_device: Option<PathBuf>,
    other_key: bool,
    raw: bool,
}

impl ChordTracker {
    pub fn key(&mut self, device: &Path, code: KeyCode, value: i32) -> Option<Command> {
        let key = (device.to_path_buf(), code);
        match value {
            PRESS => {
                self.held.insert(key);
            }
            RELEASE => {
                self.held.remove(&key);
            }
            _ => return None,
        }
        let shift = |code| matches!(code, KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT);
        match code {
            KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT => {
                self.raw |= self.alt_device.is_some() && value == PRESS;
                None
            }
            KeyCode::KEY_RIGHTALT if value == PRESS => {
                if self.alt_device.is_some() {
                    self.other_key = true;
                } else {
                    self.other_key = self
                        .held
                        .iter()
                        .any(|(path, key)| !(shift(*key) || path == device && *key == code));
                    self.raw = self.held.iter().any(|(_, key)| shift(*key));
                    self.alt_device = Some(device.to_path_buf());
                }
                None
            }
            KeyCode::KEY_RIGHTALT => {
                if self.alt_device.as_deref() != Some(device) {
                    return None;
                }
                self.alt_device = None;
                (!self.other_key).then_some(Command::Toggle { raw: self.raw })
            }
            _ => {
                self.other_key |= self.alt_device.is_some() && value == PRESS;
                (code == KeyCode::KEY_ESC && value == PRESS).then_some(Command::Cancel)
            }
        }
    }

    fn forget(&mut self, device: &Path) {
        self.held.retain(|(path, _)| path != device);
        if self.alt_device.as_deref() == Some(device) {
            self.alt_device = None;
        }
    }
}

/// Watches keyboards with Right Alt or Shift, including ones plugged in later.
pub fn spawn(commands: Sender<Command>) {
    let tracker = Arc::new(Mutex::new(ChordTracker::default()));
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
                        watch(
                            device,
                            path,
                            commands.clone(),
                            Arc::clone(&watched),
                            Arc::clone(&tracker),
                        );
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
    device.supported_keys().is_some_and(|keys| {
        keys.contains(KeyCode::KEY_RIGHTALT)
            || keys.contains(KeyCode::KEY_LEFTSHIFT)
            || keys.contains(KeyCode::KEY_RIGHTSHIFT)
    })
}

fn watch(
    mut device: Device,
    path: PathBuf,
    commands: Sender<Command>,
    watched: Arc<Mutex<HashSet<PathBuf>>>,
    tracker: Arc<Mutex<ChordTracker>>,
) {
    thread::spawn(move || {
        if let Ok(keys) = device.get_key_state() {
            let mut tracker = tracker.lock().unwrap();
            for key in keys.iter() {
                tracker.held.insert((path.clone(), key));
            }
        }
        while let Ok(events) = device.fetch_events() {
            for event in events {
                if let EventSummary::Key(_, code, value) = event.destructure()
                    && let Some(command) = tracker.lock().unwrap().key(&path, code, value)
                    && commands.send(command).is_err()
                {
                    return;
                }
            }
        }
        // The keyboard went away; forget it so a replug is picked up by the next scan.
        tracker.lock().unwrap().forget(&path);
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
            .filter_map(|&(code, value)| tracker.key(Path::new("a"), code, value))
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
            (KeyCode::KEY_C, RELEASE),
            (KeyCode::KEY_RIGHTALT, PRESS),
            (KeyCode::KEY_RIGHTALT, RELEASE),
        ]);
        assert_eq!(commands, [Command::Toggle { raw: false }]);
    }

    #[test]
    fn both_shifts_remain_raw_after_one_is_released() {
        assert_eq!(
            run(&[
                (KeyCode::KEY_LEFTSHIFT, PRESS),
                (KeyCode::KEY_RIGHTSHIFT, PRESS),
                (KeyCode::KEY_LEFTSHIFT, RELEASE),
                (KeyCode::KEY_RIGHTALT, PRESS),
                (KeyCode::KEY_RIGHTALT, RELEASE),
            ]),
            [Command::Toggle { raw: true }]
        );
    }

    #[test]
    fn held_keys_and_escape_prevent_a_tap() {
        assert!(
            run(&[
                (KeyCode::KEY_LEFTCTRL, PRESS),
                (KeyCode::KEY_RIGHTALT, PRESS),
                (KeyCode::KEY_RIGHTALT, RELEASE)
            ])
            .is_empty()
        );
        assert_eq!(
            run(&[
                (KeyCode::KEY_RIGHTALT, PRESS),
                (KeyCode::KEY_ESC, PRESS),
                (KeyCode::KEY_RIGHTALT, RELEASE)
            ]),
            [Command::Cancel]
        );
    }

    #[test]
    fn keyboards_share_modifiers_and_release_their_own_keys() {
        let mut tracker = ChordTracker::default();
        let a = Path::new("a");
        let b = Path::new("b");
        tracker.key(a, KeyCode::KEY_LEFTSHIFT, PRESS);
        tracker.key(b, KeyCode::KEY_LEFTSHIFT, PRESS);
        tracker.key(a, KeyCode::KEY_LEFTSHIFT, RELEASE);
        tracker.key(a, KeyCode::KEY_RIGHTALT, PRESS);
        assert_eq!(
            tracker.key(a, KeyCode::KEY_RIGHTALT, RELEASE),
            Some(Command::Toggle { raw: true })
        );
        tracker.forget(b);
        tracker.key(a, KeyCode::KEY_RIGHTALT, PRESS);
        assert_eq!(
            tracker.key(a, KeyCode::KEY_RIGHTALT, RELEASE),
            Some(Command::Toggle { raw: false })
        );
    }

    #[test]
    fn escape_cancels() {
        assert_eq!(
            run(&[(KeyCode::KEY_ESC, PRESS), (KeyCode::KEY_ESC, RELEASE)]),
            [Command::Cancel]
        );
    }
}
