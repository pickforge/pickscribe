use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

pub const DEFAULT_INSTRUCTIONS: &str = "Rewrite this dictated text so it is clean, natural, and ready to paste.\nKeep the original language. If the text is Portuguese, use natural Brazilian Portuguese.\nFix punctuation, grammar, casing, and obvious speech-to-text mistakes.\nDo not add explanations.\nReturn only the final text.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub sounds: bool,
    pub float_button: bool,
    pub typing_wpm: u32,
    pub keep_audio: bool,
    pub crash_reports: bool,
    /// When true, no text ever leaves this machine: only loopback cleanup
    /// endpoints are allowed, everything else is skipped.
    pub local_only: bool,
    /// "system", "dark", or "light".
    pub theme: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            sounds: true,
            float_button: true,
            typing_wpm: 40,
            keep_audio: false,
            crash_reports: true,
            local_only: false,
            theme: "system".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttConfig {
    /// Path to a ggml model file; empty means auto-detect.
    pub model_path: String,
    /// "auto", "en", "pt", ...
    pub language: String,
    /// Optional PipeWire target node for pw-record.
    pub audio_target: String,
    /// Recorder command, default pw-record.
    pub recorder: String,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            model_path: String::new(),
            language: "auto".into(),
            audio_target: String::new(),
            recorder: "pw-record".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IncrementalConfig {
    pub enabled: bool,
    pub cleanup_segments: bool,
    pub target_ms: u64,
    pub max_ms: u64,
    pub overlap_ms: u64,
    pub max_queue: usize,
}

impl Default for IncrementalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cleanup_segments: false,
            target_ms: 5_000,
            max_ms: 10_000,
            overlap_ms: 1_500,
            max_queue: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanupConfig {
    /// "auto", "deepseek", "openai", "ollama", "none"
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    /// Stored key; env vars and ~/.config/pickscribe/env take precedence.
    pub api_key: String,
    pub temperature: f32,
    pub timeout_secs: u64,
    /// "auto", "enabled", "disabled" (DeepSeek thinking mode)
    pub thinking: String,
    /// Empty means built-in default instructions.
    pub instructions: String,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            provider: "auto".into(),
            model: String::new(),
            endpoint: String::new(),
            api_key: String::new(),
            temperature: 0.2,
            timeout_secs: 30,
            thinking: "disabled".into(),
            instructions: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PasteConfig {
    /// "auto", "hotkey", "type", "none"
    pub method: String,
    /// "ctrl-v" or "ctrl-shift-v"
    pub chord: String,
    pub delay_ms: u64,
    pub copy_to_clipboard: bool,
}

impl Default for PasteConfig {
    fn default() -> Self {
        Self {
            method: "auto".into(),
            chord: "ctrl-v".into(),
            delay_ms: 150,
            copy_to_clipboard: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub stt: SttConfig,
    pub incremental: IncrementalConfig,
    pub cleanup: CleanupConfig,
    pub paste: PasteConfig,
}

pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("pickscribe");
        }
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
        .join(".config")
        .join("pickscribe")
}

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("pickscribe");
        }
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
        .join(".local")
        .join("share")
        .join("pickscribe")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

impl AppConfig {
    pub fn load() -> Self {
        Self::load_from_path(&config_path())
    }

    fn load_from_path(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        self.save_to_path(&config_path())
    }

    fn save_to_path(&self, path: &Path) -> Result<()> {
        self.save_to_path_with(path, |temp, destination| {
            temp.persist(destination)
                .map_err(|err| err.error)
                .context("replacing config.toml")?;
            Ok(())
        })
    }

    fn save_to_path_with(
        &self,
        path: &Path,
        persist: impl FnOnce(NamedTempFile, &Path) -> Result<()>,
    ) -> Result<()> {
        let raw = toml::to_string_pretty(self)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = NamedTempFile::new_in(parent).context("creating temporary config file")?;
        temp.write_all(raw.as_bytes())
            .context("writing temporary config file")?;
        temp.as_file()
            .sync_all()
            .context("syncing temporary config file")?;
        persist(temp, path)
    }

    /// Resolve the effective API key for the configured provider:
    /// process env > ~/.config/pickscribe/env > config.toml.
    pub fn resolve_api_key(&self, provider: &str) -> Option<String> {
        let env_file = read_env_file();
        self.resolve_api_key_from_sources(provider, &env_file, lookup_env)
    }

    fn resolve_api_key_from_sources(
        &self,
        provider: &str,
        env_file: &HashMap<String, String>,
        process_env: impl Fn(&str) -> Option<String>,
    ) -> Option<String> {
        let lookup = |name: &str| -> Option<String> {
            process_env(name).or_else(|| env_file.get(name).cloned())
        };
        let key = match provider {
            "deepseek" => lookup("DEEPSEEK_API_KEY").or_else(|| lookup("PICKSCRIBE_API_KEY")),
            "openai" => lookup("OPENAI_API_KEY").or_else(|| lookup("PICKSCRIBE_API_KEY")),
            "ollama" => lookup("OLLAMA_API_KEY").or_else(|| lookup("PICKSCRIBE_API_KEY")),
            _ => lookup("PICKSCRIBE_API_KEY"),
        };
        key.or_else(|| {
            if self.cleanup.api_key.is_empty() {
                None
            } else {
                Some(self.cleanup.api_key.clone())
            }
        })
    }
}

/// Parse `export KEY="value"` / `KEY=value` lines from ~/.config/pickscribe/env
/// (the file sourced by the CLI wrappers) so the GUI shares credentials.
pub fn read_env_file() -> HashMap<String, String> {
    let path = config_dir().join("env");
    let Ok(raw) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    parse_env_file(&raw)
}

fn parse_env_file(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let mut value = value.trim().to_string();
        if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
            || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
        {
            value = value[1..value.len() - 1].to_string();
        }
        map.insert(key.to_string(), value);
    }
    map
}

fn lookup_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn read_env_file_parses_supported_shell_assignments() {
        let env_file = parse_env_file(
            r#"
                # ignored
                export DEEPSEEK_API_KEY="deepseek file"
                OPENAI_API_KEY='openai file'
                PICKSCRIBE_API_KEY= fallback
                BAD-KEY=no
                MISSING
                EMPTY_KEY=
            "#,
        );

        assert_eq!(
            env_file.get("DEEPSEEK_API_KEY").map(String::as_str),
            Some("deepseek file")
        );
        assert_eq!(
            env_file.get("OPENAI_API_KEY").map(String::as_str),
            Some("openai file")
        );
        assert_eq!(
            env_file.get("PICKSCRIBE_API_KEY").map(String::as_str),
            Some("fallback")
        );
        assert_eq!(env_file.get("EMPTY_KEY").map(String::as_str), Some(""));
        assert!(!env_file.contains_key("BAD-KEY"));
        assert!(!env_file.contains_key("MISSING"));
    }

    #[test]
    fn resolve_api_key_prefers_process_env_then_env_file_then_config() {
        let env_file = HashMap::from([
            ("DEEPSEEK_API_KEY".to_string(), "file-deepseek".to_string()),
            ("PICKSCRIBE_API_KEY".to_string(), "file-generic".to_string()),
        ]);
        let mut cfg = AppConfig::default();
        cfg.cleanup.api_key = "config-key".into();

        assert_eq!(
            cfg.resolve_api_key_from_sources("deepseek", &env_file, |key| {
                (key == "DEEPSEEK_API_KEY").then(|| "process-deepseek".to_string())
            })
            .as_deref(),
            Some("process-deepseek")
        );

        assert_eq!(
            cfg.resolve_api_key_from_sources("deepseek", &env_file, |_| None)
                .as_deref(),
            Some("file-deepseek")
        );
        assert_eq!(
            cfg.resolve_api_key_from_sources("openai", &env_file, |_| None)
                .as_deref(),
            Some("file-generic")
        );

        assert_eq!(
            cfg.resolve_api_key_from_sources("openai", &HashMap::new(), |_| None)
                .as_deref(),
            Some("config-key")
        );
    }

    #[test]
    fn incremental_config_defaults_to_live_local_segments_only() {
        let cfg = AppConfig::default();

        assert!(cfg.incremental.enabled);
        assert!(!cfg.incremental.cleanup_segments);
        assert_eq!(cfg.incremental.target_ms, 5_000);
        assert_eq!(cfg.incremental.max_ms, 10_000);
        assert_eq!(cfg.incremental.overlap_ms, 1_500);
        assert_eq!(cfg.incremental.max_queue, 2);
    }

    #[test]
    fn incremental_config_deserializes_partial_toml() {
        let cfg: AppConfig = toml::from_str(
            r#"
            [incremental]
            enabled = true
            target_ms = 3000
            "#,
        )
        .unwrap();

        assert!(cfg.incremental.enabled);
        assert!(!cfg.incremental.cleanup_segments);
        assert_eq!(cfg.incremental.target_ms, 3_000);
        assert_eq!(cfg.incremental.max_ms, 10_000);
    }

    #[test]
    fn crash_reports_default_to_enabled_when_absent() {
        let cfg: AppConfig = toml::from_str(
            r#"
            [general]
            local_only = true
            "#,
        )
        .unwrap();

        assert!(cfg.general.crash_reports);
    }

    #[test]
    fn crash_reports_round_trip_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = AppConfig::default();
        cfg.general.crash_reports = false;

        cfg.save_to_path(&path).unwrap();
        let loaded = AppConfig::load_from_path(&path);

        assert!(!loaded.general.crash_reports);
    }

    #[test]
    fn failed_atomic_replace_preserves_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = AppConfig::default();
        original.save_to_path(&path).unwrap();
        let original_bytes = fs::read(&path).unwrap();
        let mut replacement = original.clone();
        replacement.general.local_only = true;

        let error = replacement
            .save_to_path_with(&path, |_temp, _destination| {
                anyhow::bail!("simulated rename failure")
            })
            .unwrap_err();

        assert!(error.to_string().contains("simulated rename failure"));
        assert_eq!(fs::read(&path).unwrap(), original_bytes);
        assert!(!AppConfig::load_from_path(&path).general.local_only);
    }
}
