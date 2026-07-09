use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
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
    pub source_file: Option<String>,
    pub segments_json: Option<String>,
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
    pub source_file: Option<String>,
    pub segments_json: Option<String>,
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

fn history_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        created_at: row.get(1)?,
        duration_ms: row.get(2)?,
        raw_text: row.get(3)?,
        cleaned_text: row.get(4)?,
        provider: row.get(5)?,
        model: row.get(6)?,
        language: row.get(7)?,
        source_file: row.get(8)?,
        segments_json: row.get(9)?,
        word_count: row.get(10)?,
    })
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
                source_file TEXT,
                segments_json TEXT,
                word_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_transcriptions_created
                ON transcriptions(created_at DESC);",
        )?;
        let columns = {
            let mut statement = conn.prepare("PRAGMA table_info(transcriptions)")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !columns.iter().any(|column| column == "source_file") {
            conn.execute(
                "ALTER TABLE transcriptions ADD COLUMN source_file TEXT",
                [],
            )?;
        }
        if !columns.iter().any(|column| column == "segments_json") {
            conn.execute(
                "ALTER TABLE transcriptions ADD COLUMN segments_json TEXT",
                [],
            )?;
        }
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
                (created_at, duration_ms, raw_text, cleaned_text, provider, model, language,
                 source_file, segments_json, word_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                now_unix(),
                entry.duration_ms,
                entry.raw_text,
                entry.cleaned_text,
                entry.provider,
                entry.model,
                entry.language,
                entry.source_file,
                entry.segments_json,
                words,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list(&self, search: &str, limit: i64, offset: i64) -> Result<Vec<HistoryEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::new();
        if search.trim().is_empty() {
            let mut stmt = conn.prepare(
                "SELECT id, created_at, duration_ms, raw_text, cleaned_text,
                        provider, model, language, source_file, segments_json, word_count
                 FROM transcriptions ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
            )?;
            let rows = stmt.query_map(params![limit, offset], history_entry_from_row)?;
            for row in rows {
                out.push(row?);
            }
        } else {
            let pattern = format!("%{}%", search.trim());
            let mut stmt = conn.prepare(
                "SELECT id, created_at, duration_ms, raw_text, cleaned_text,
                        provider, model, language, source_file, segments_json, word_count
                 FROM transcriptions
                 WHERE raw_text LIKE ?1 OR cleaned_text LIKE ?1
                 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
            )?;
            let rows = stmt.query_map(params![pattern, limit, offset], history_entry_from_row)?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn get(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, created_at, duration_ms, raw_text, cleaned_text,
                    provider, model, language, source_file, segments_json, word_count
             FROM transcriptions WHERE id = ?1",
            params![id],
            history_entry_from_row,
        )
        .optional()
        .context("loading history entry")
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
                 FROM transcriptions WHERE source_file IS NULL",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;

        let mut days = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT date(created_at, 'unixepoch', 'localtime') AS day,
                    COALESCE(SUM(word_count), 0),
                    COUNT(*)
             FROM transcriptions
             WHERE source_file IS NULL
               AND created_at >= unixepoch('now', '-13 days', 'start of day')
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pickscribe-{name}-{id}.db"))
    }

    #[test]
    fn metrics_aggregate_sessions_words_durations_and_recent_days() -> Result<()> {
        let path = temp_db_path("history-metrics");
        let db = HistoryDb::open(&path)?;

        db.insert(&NewEntry {
            duration_ms: 1_000,
            raw_text: "one two three".into(),
            cleaned_text: None,
            provider: "none".into(),
            model: String::new(),
            language: "en".into(),
            source_file: None,
            segments_json: None,
        })?;
        db.insert(&NewEntry {
            duration_ms: 2_000,
            raw_text: "raw text ignored".into(),
            cleaned_text: Some("four five six seven".into()),
            provider: "ollama".into(),
            model: "qwen".into(),
            language: "en".into(),
            source_file: None,
            segments_json: None,
        })?;

        let metrics = db.metrics(70)?;

        assert_eq!(metrics.sessions, 2);
        assert_eq!(metrics.words, 7);
        assert_eq!(metrics.speaking_ms, 3_000);
        assert_eq!(metrics.longest_session_ms, 2_000);
        assert_eq!(metrics.avg_words_per_session, 3.5);
        assert!((metrics.minutes_saved - 0.05).abs() < 0.0001);
        assert_eq!(metrics.days.iter().map(|day| day.sessions).sum::<i64>(), 2);
        assert_eq!(metrics.days.iter().map(|day| day.words).sum::<i64>(), 7);

        let entries = db.list("", 10, 0)?;
        assert!(entries.iter().any(|entry| entry.word_count == 3));
        assert!(entries.iter().any(|entry| entry.word_count == 4));

        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn metrics_exclude_file_transcriptions() -> Result<()> {
        let path = temp_db_path("history-file-metrics");
        let db = HistoryDb::open(&path)?;

        db.insert(&NewEntry {
            duration_ms: 1_000,
            raw_text: "one two three".into(),
            cleaned_text: None,
            provider: "none".into(),
            model: String::new(),
            language: "en".into(),
            source_file: None,
            segments_json: None,
        })?;
        db.insert(&NewEntry {
            duration_ms: 7_200_000,
            raw_text: "video transcript".into(),
            cleaned_text: None,
            provider: "none".into(),
            model: String::new(),
            language: "en".into(),
            source_file: Some("meeting.mp4".into()),
            segments_json: None,
        })?;

        let metrics = db.metrics(60)?;

        assert_eq!(metrics.sessions, 1);
        assert_eq!(metrics.words, 3);
        assert_eq!(metrics.speaking_ms, 1_000);
        assert_eq!(metrics.longest_session_ms, 1_000);
        assert_eq!(metrics.avg_words_per_session, 3.0);
        assert!((metrics.minutes_saved - (2.0 / 60.0)).abs() < 0.0001);
        assert_eq!(metrics.days.iter().map(|day| day.sessions).sum::<i64>(), 1);
        assert_eq!(metrics.days.iter().map(|day| day.words).sum::<i64>(), 3);

        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn metrics_handles_zero_typing_wpm_without_dividing_by_zero() -> Result<()> {
        let path = temp_db_path("history-zero-wpm");
        let db = HistoryDb::open(&path)?;

        db.insert(&NewEntry {
            duration_ms: 60_000,
            raw_text: "one two".into(),
            cleaned_text: None,
            provider: "none".into(),
            model: String::new(),
            language: "en".into(),
            source_file: None,
            segments_json: None,
        })?;

        let metrics = db.metrics(0)?;

        assert_eq!(metrics.typing_wpm, 0);
        assert_eq!(metrics.words, 2);
        assert_eq!(metrics.minutes_saved, 1.0);

        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn open_migrates_legacy_schema_and_round_trips_file_fields() -> Result<()> {
        let path = temp_db_path("history-legacy-media");
        let conn = Connection::open(&path)?;
        conn.execute_batch(
            "CREATE TABLE transcriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                raw_text TEXT NOT NULL,
                cleaned_text TEXT,
                provider TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                language TEXT NOT NULL DEFAULT '',
                word_count INTEGER NOT NULL DEFAULT 0
            );",
        )?;
        drop(conn);

        let db = HistoryDb::open(&path)?;
        let id = db.insert(&NewEntry {
            duration_ms: 3_200,
            raw_text: "legacy transcript".into(),
            cleaned_text: None,
            provider: "none".into(),
            model: String::new(),
            language: "en".into(),
            source_file: Some("/home/me/recording.mp4".into()),
            segments_json: Some("[{\"start_ms\":0}]".into()),
        })?;

        let entries = db.list("", 10, 0)?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].source_file.as_deref(), Some("/home/me/recording.mp4"));
        assert_eq!(entries[0].segments_json.as_deref(), Some("[{\"start_ms\":0}]"));
        assert_eq!(db.get(id)?.map(|entry| entry.id), Some(id));
        assert!(db.get(id + 1)?.is_none());

        drop(db);
        std::fs::remove_file(path).ok();
        Ok(())
    }

    #[test]
    fn fresh_database_round_trips_file_fields() -> Result<()> {
        let path = temp_db_path("history-fresh-media");
        let db = HistoryDb::open(&path)?;
        let id = db.insert(&NewEntry {
            duration_ms: 8_000,
            raw_text: "file transcript".into(),
            cleaned_text: Some("clean file transcript".into()),
            provider: "none".into(),
            model: "ggml-small.bin".into(),
            language: "pt".into(),
            source_file: Some("meeting.webm".into()),
            segments_json: Some("[{\"start_ms\":0,\"end_ms\":8000}]".into()),
        })?;

        let entries = db.list("", 10, 0)?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].source_file.as_deref(), Some("meeting.webm"));
        assert_eq!(
            entries[0].segments_json.as_deref(),
            Some("[{\"start_ms\":0,\"end_ms\":8000}]")
        );

        drop(db);
        std::fs::remove_file(path).ok();
        Ok(())
    }
}
