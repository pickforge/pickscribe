use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const API_KEY_VAR: &str = "ANTHROPIC_API_KEY";

/// Settings from `~/.config/pickscribe/pickscribe.toml`. Every field has a default, so the
/// file is optional.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub whisper_model: PathBuf,
    /// Languages you speak. With more than one, PickScribe detects which of them each
    /// recording uses.
    pub languages: Vec<String>,
    pub cleanup_model: String,
    /// `low`, `medium`, `high`, `xhigh`, or `max`.
    pub cleanup_effort: String,
    /// Lets the cleanup model think before it answers. Slower, and rarely needed.
    pub cleanup_thinking: bool,
    /// Extra cleanup rules, appended to the built-in instructions.
    pub cleanup_instructions: String,
    pub max_recording_secs: u64,
    /// How many recent dictations to keep. 0 keeps none.
    pub history_limit: usize,
    /// Review every this many AI dictations for misheard words. 0 turns the review off.
    pub review_every: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            whisper_model: data_dir().join("models/ggml-large-v3-turbo.bin"),
            languages: vec!["pt".into(), "en".into()],
            cleanup_model: "claude-haiku-5-5".into(),
            cleanup_effort: "low".into(),
            cleanup_thinking: false,
            cleanup_instructions: String::new(),
            max_recording_secs: 300,
            history_limit: 500,
            review_every: 10,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_dir().join("pickscribe.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("invalid {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if !EFFORTS.contains(&self.cleanup_effort.as_str()) {
            bail!("cleanup_effort must be one of {}", EFFORTS.join(", "));
        }
        // The API rejects disabled thinking at the two highest levels.
        if !self.cleanup_thinking && matches!(self.cleanup_effort.as_str(), "xhigh" | "max") {
            bail!(
                "cleanup_effort {} needs cleanup_thinking = true",
                self.cleanup_effort
            );
        }
        Ok(())
    }
}

pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Reads the cleanup key on every use, so adding it does not need a restart. The key file
/// comes first, so a dedicated, spend-capped key wins over a general shell variable.
pub fn api_key() -> Option<String> {
    let from_file = std::fs::read_to_string(config_dir().join("env"))
        .ok()
        .and_then(|text| env_file_value(&text, API_KEY_VAR));
    from_file
        .or_else(|| std::env::var(API_KEY_VAR).ok())
        .filter(|key| !key.trim().is_empty())
}

fn env_file_value(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (key, value) = line.split_once('=')?;
        (key.trim() == name).then(|| unquote(value.trim()).to_string())
    })
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// Writes the file atomically with mode 600.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("path has no parent")?;
    std::fs::create_dir_all(dir)?;
    let temp = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .with_context(|| format!("cannot write {}", temp.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&temp, path).with_context(|| format!("cannot write {}", path.display()))
}

pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config").join("pickscribe")
}

pub fn data_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share").join("pickscribe")
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| Path::new("/").to_path_buf(), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_uses_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(config.languages, ["pt", "en"]);
        assert_eq!(config.cleanup_model, "claude-haiku-5-5");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(Config::parse("lenguages = [\"pt\"]").is_err());
    }

    #[test]
    fn effort_must_be_known_and_fit_the_thinking_mode() {
        assert!(Config::parse("cleanup_effort = \"fast\"").is_err());
        assert!(Config::parse("cleanup_effort = \"max\"").is_err());
        assert!(Config::parse("cleanup_effort = \"max\"\ncleanup_thinking = true").is_ok());
        assert!(Config::parse("cleanup_effort = \"high\"").is_ok());
    }

    #[test]
    fn env_file_values_accept_export_and_quotes() {
        let text = "# keys\nDEEPSEEK_API_KEY=other\nexport ANTHROPIC_API_KEY=\"sk-test\"\n";
        assert_eq!(
            env_file_value(text, API_KEY_VAR).as_deref(),
            Some("sk-test")
        );
        assert_eq!(
            env_file_value("ANTHROPIC_API_KEY='x'", API_KEY_VAR).as_deref(),
            Some("x")
        );
        assert_eq!(env_file_value("OTHER=1", API_KEY_VAR), None);
    }
}
