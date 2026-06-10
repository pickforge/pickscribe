use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub created_at: i64,
    pub duration_ms: i64,
    pub raw_text: String,
    pub cleaned_text: Option<String>,
    pub provider: String,
    pub model: String,
    pub language: String,
    pub word_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewEntry {
    pub duration_ms: i64,
    pub raw_text: String,
    pub cleaned_text: Option<String>,
    pub provider: String,
    pub model: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayStat {
    pub day: String,
    pub words: i64,
    pub sessions: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub sessions: i64,
    pub words: i64,
    pub speaking_ms: i64,
    /// Estimated minutes saved vs typing at `typing_wpm`.
    pub minutes_saved: f64,
    pub typing_wpm: u32,
    pub avg_words_per_session: f64,
    pub longest_session_ms: i64,
    pub days: Vec<DayStat>,
}

pub struct HistoryDb {
    conn: Mutex<Connection>,
}

pub fn count_words(text: &str) -> i64 {
    text.split_whitespace().count() as i64
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl HistoryDb {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path).context("opening history database")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS transcriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                raw_text TEXT NOT NULL,
                cleaned_text TEXT,
                provider TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                language TEXT NOT NULL DEFAULT '',
                word_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_transcriptions_created
                ON transcriptions(created_at DESC);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&crate::config::data_dir().join("pickscribe.db"))
    }

    pub fn insert(&self, entry: &NewEntry) -> Result<i64> {
        let words = count_words(
            entry
                .cleaned_text
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(&entry.raw_text),
        );
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO transcriptions
                (created_at, duration_ms, raw_text, cleaned_text, provider, model, language, word_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                now_unix(),
                entry.duration_ms,
                entry.raw_text,
                entry.cleaned_text,
                entry.provider,
                entry.model,
                entry.language,
                words,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list(&self, search: &str, limit: i64, offset: i64) -> Result<Vec<HistoryEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::new();
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<HistoryEntry> {
            Ok(HistoryEntry {
                id: row.get(0)?,
                created_at: row.get(1)?,
                duration_ms: row.get(2)?,
                raw_text: row.get(3)?,
                cleaned_text: row.get(4)?,
                provider: row.get(5)?,
                model: row.get(6)?,
                language: row.get(7)?,
                word_count: row.get(8)?,
            })
        };
        if search.trim().is_empty() {
            let mut stmt = conn.prepare(
                "SELECT id, created_at, duration_ms, raw_text, cleaned_text,
                        provider, model, language, word_count
                 FROM transcriptions ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
            )?;
            let rows = stmt.query_map(params![limit, offset], map_row)?;
            for row in rows {
                out.push(row?);
            }
        } else {
            let pattern = format!("%{}%", search.trim());
            let mut stmt = conn.prepare(
                "SELECT id, created_at, duration_ms, raw_text, cleaned_text,
                        provider, model, language, word_count
                 FROM transcriptions
                 WHERE raw_text LIKE ?1 OR cleaned_text LIKE ?1
                 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
            )?;
            let rows = stmt.query_map(params![pattern, limit, offset], map_row)?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM transcriptions WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM transcriptions", [])?;
        Ok(())
    }

    pub fn metrics(&self, typing_wpm: u32) -> Result<Metrics> {
        let conn = self.conn.lock().unwrap();
        let (sessions, words, speaking_ms, longest_session_ms): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(word_count), 0),
                        COALESCE(SUM(duration_ms), 0),
                        COALESCE(MAX(duration_ms), 0)
                 FROM transcriptions",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;

        let mut days = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT date(created_at, 'unixepoch', 'localtime') AS day,
                    COALESCE(SUM(word_count), 0),
                    COUNT(*)
             FROM transcriptions
             WHERE created_at >= unixepoch('now', '-13 days', 'start of day')
             GROUP BY day ORDER BY day ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DayStat {
                day: row.get(0)?,
                words: row.get(1)?,
                sessions: row.get(2)?,
            })
        })?;
        for row in rows {
            days.push(row?);
        }

        let wpm = typing_wpm.max(1) as f64;
        let typing_minutes = words as f64 / wpm;
        let speaking_minutes = speaking_ms as f64 / 60_000.0;
        let minutes_saved = (typing_minutes - speaking_minutes).max(0.0);
        let avg_words_per_session = if sessions > 0 {
            words as f64 / sessions as f64
        } else {
            0.0
        };

        Ok(Metrics {
            sessions,
            words,
            speaking_ms,
            minutes_saved,
            typing_wpm,
            avg_words_per_session,
            longest_session_ms,
            days,
        })
    }
}
