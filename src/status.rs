//! Live daemon state for `pickscribe watch`, which the Omarchy bar widget follows. Each change
//! sends a full JSON snapshot line to every watcher. Snapshots never hold transcript text.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

use crate::config;

/// A watcher that cannot take a line this fast is dropped, so it never stalls dictation.
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Idle,
    Listening,
    Thinking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Timings {
    pub audio_ms: u64,
    pub transcribe_ms: u64,
    pub cleanup_ms: Option<u64>,
    /// From the stop tap to the paste.
    pub total_ms: u64,
    pub mode: crate::history::Mode,
}

#[derive(Debug, Clone, Serialize)]
struct Snapshot {
    state: State,
    raw: bool,
    version: &'static str,
    commit: &'static str,
    api_key: bool,
    history_rev: u64,
    dictionary_rev: u64,
    last: Option<Timings>,
}

pub struct Hub {
    inner: Mutex<Inner>,
}

struct Inner {
    snapshot: Snapshot,
    watchers: Vec<UnixStream>,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                snapshot: Snapshot {
                    state: State::Idle,
                    raw: false,
                    version: env!("CARGO_PKG_VERSION"),
                    commit: env!("PICKSCRIBE_COMMIT"),
                    api_key: false,
                    history_rev: 0,
                    dictionary_rev: 0,
                    last: None,
                },
                watchers: Vec::new(),
            }),
        }
    }
}

impl Hub {
    pub fn set_state(&self, state: State, raw: bool) {
        self.update(|snapshot| {
            snapshot.state = state;
            snapshot.raw = raw;
        });
    }

    pub fn finished(&self, timings: Option<Timings>, history_changed: bool) {
        self.update(|snapshot| {
            snapshot.state = State::Idle;
            if timings.is_some() {
                snapshot.last = timings;
            }
            if history_changed {
                snapshot.history_rev += 1;
            }
        });
    }

    pub fn history_changed(&self) {
        self.update(|snapshot| snapshot.history_rev += 1);
    }

    pub fn dictionary_changed(&self) {
        self.update(|snapshot| snapshot.dictionary_rev += 1);
    }

    /// Sends the current snapshot, then every later one, to `stream`.
    pub fn watch(&self, stream: UnixStream) {
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let mut inner = self.lock();
        inner.snapshot.api_key = config::api_key().is_some();
        let line = line(&inner.snapshot);
        let mut stream = stream;
        if stream.write_all(line.as_bytes()).is_ok() {
            inner.watchers.push(stream);
        }
    }

    fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        let mut inner = self.lock();
        change(&mut inner.snapshot);
        // The key file can change at any time; this keeps the panel's hint current.
        inner.snapshot.api_key = config::api_key().is_some();
        let line = line(&inner.snapshot);
        inner
            .watchers
            .retain_mut(|stream| stream.write_all(line.as_bytes()).is_ok());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn line(snapshot: &Snapshot) -> String {
    let mut line = serde_json::to_string(snapshot).unwrap_or_default();
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_matches_the_widget_contract() {
        let hub = Hub::default();
        hub.set_state(State::Listening, true);
        let value: serde_json::Value = serde_json::from_str(&line(&hub.lock().snapshot)).unwrap();
        assert_eq!(value["state"], "listening");
        assert_eq!(value["raw"], true);
        assert_eq!(value["last"], serde_json::Value::Null);
        assert!(value["history_rev"].is_u64());
    }

    #[test]
    fn watchers_receive_each_change() {
        let hub = Hub::default();
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        hub.watch(ours);
        hub.history_changed();
        drop(hub);
        let mut text = String::new();
        std::io::Read::read_to_string(&mut theirs, &mut text).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("\"history_rev\":1"));
    }
}
