//! Recent dictations, kept so text is never lost when no field had focus. Stored as JSON lines
//! in `~/.local/share/pickscribe/history.jsonl`, readable only by the user. Audio is never kept.

use std::fs::File;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    /// Unix seconds.
    pub at: u64,
    pub mode: Mode,
    /// The text that was pasted, or would have been.
    pub text: String,
    /// What Whisper heard, before the AI cleanup.
    pub transcript: String,
    /// False when pasting failed, so the text only reached the clipboard.
    pub delivered: bool,
    pub audio_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Ai,
    Raw,
}

impl Item {
    pub fn new(mode: Mode, text: String, transcript: String, audio_ms: u64) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            id: now.as_millis().to_string(),
            at: now.as_secs(),
            mode,
            text,
            transcript,
            delivered: false,
            audio_ms,
        }
    }
}

/// Newest last, as stored.
pub fn load() -> Result<Vec<Item>> {
    let path = path();
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(parse(&text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err).with_context(|| format!("cannot read {}", path.display())),
    }
}

fn parse(text: &str) -> Vec<Item> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Appends an item and drops the oldest ones beyond `limit`.
pub fn append(item: &Item, limit: usize) -> Result<()> {
    edit(|items| {
        items.push(item.clone());
        let excess = items.len().saturating_sub(limit);
        items.drain(..excess);
    })
}

/// Applies `change` to the stored items under a lock, so the daemon and the CLI never
/// overwrite each other.
pub fn edit(change: impl FnOnce(&mut Vec<Item>)) -> Result<()> {
    let path = path();
    let dir = path.parent().context("history path has no parent")?;
    std::fs::create_dir_all(dir)?;
    let lock = File::create(dir.join("history.lock"))?;
    lock.lock()?;
    let mut items = load()?;
    change(&mut items);
    let mut text = String::new();
    for item in &items {
        text.push_str(&serde_json::to_string(item)?);
        text.push('\n');
    }
    config::write_private(&path, text.as_bytes())
}

pub fn path() -> PathBuf {
    config::data_dir().join("history.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_round_trip_and_bad_lines_are_skipped() {
        let item = Item::new(Mode::Ai, "Olá.".into(), "olá".into(), 1200);
        let text = format!("{}\nnot json\n", serde_json::to_string(&item).unwrap());
        assert_eq!(parse(&text), [item]);
    }

    #[test]
    fn mode_uses_lowercase_names() {
        assert_eq!(serde_json::to_string(&Mode::Raw).unwrap(), "\"raw\"");
    }
}
