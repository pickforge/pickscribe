//! Terms to spell exactly, and corrections for words Whisper keeps mishearing. Each line of
//! `~/.config/pickscribe/dictionary.txt` holds a term (`Pickforge`) or a correction
//! (`Bitforge -> Pickforge`). The file is read for every dictation, so edits apply at once.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::config;

const ARROW: &str = "->";
const MAX_LEN: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// The spelling to use.
    pub text: String,
    /// What Whisper heard instead, for a correction.
    pub heard: Option<String>,
}

impl Entry {
    pub fn new(text: &str, heard: Option<&str>) -> Result<Self> {
        let text = clean_field(text, "term")?;
        let heard = heard.map(|heard| clean_field(heard, "heard")).transpose()?;
        if heard
            .as_deref()
            .is_some_and(|heard| heard.eq_ignore_ascii_case(&text))
        {
            bail!("the heard word and the term are the same");
        }
        Ok(Self { text, heard })
    }

    fn parse(line: &str) -> Option<Self> {
        match line.split_once(ARROW) {
            Some((heard, text)) => Self::new(text, Some(heard)).ok(),
            None => Self::new(line, None).ok(),
        }
    }

    fn line(&self) -> String {
        match &self.heard {
            Some(heard) => format!("{heard} {ARROW} {}", self.text),
            None => self.text.clone(),
        }
    }
}

fn clean_field(value: &str, name: &str) -> Result<String> {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().any(char::is_control) {
        bail!("the {name} cannot contain control characters");
    }
    if value.is_empty() {
        bail!("the {name} is empty");
    }
    if value.contains(ARROW) || value.starts_with('#') {
        bail!("the {name} cannot contain \"{ARROW}\" or start with \"#\"");
    }
    if value.chars().count() > MAX_LEN {
        bail!("the {name} is longer than {MAX_LEN} characters");
    }
    Ok(value)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dictionary {
    pub entries: Vec<Entry>,
}

impl Dictionary {
    pub fn load() -> Result<Self> {
        Self::load_at(&path())
    }

    fn load_at(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    /// Lines that do not parse are skipped, so a typo never stops dictation.
    fn parse(text: &str) -> Self {
        let entries = text.lines().filter_map(Entry::parse).collect();
        Self { entries }
    }

    fn save(&self, path: &Path) -> Result<()> {
        let original = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::from(
                "# PickScribe dictionary: one term per line, or \"heard -> meant\" for a correction.\n",
            ),
            Err(err) => return Err(err).context("cannot read dictionary"),
        };
        let mut remaining = self.entries.clone();
        let mut text = String::new();
        for line in original.split_inclusive('\n') {
            if let Some(entry) = Entry::parse(line) {
                let Some(index) = remaining.iter().position(|e| e == &entry) else {
                    continue;
                };
                remaining.remove(index);
            }
            text.push_str(line);
        }
        for entry in remaining {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&entry.line());
            text.push('\n');
        }
        config::write_private(path, text.as_bytes())
    }

    /// Locks the whole edit so all dictionary writers keep each other's entries.
    pub fn edit<T>(change: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        Self::edit_at(&path(), change)
    }

    fn edit_at<T>(path: &Path, change: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        std::fs::create_dir_all(path.parent().context("dictionary path has no parent")?)?;
        let lock = File::create(path.with_extension("lock"))?;
        lock.lock()?;
        let mut dictionary = Self::load_at(path)?;
        let original = dictionary.clone();
        let value = change(&mut dictionary)?;
        if dictionary != original {
            dictionary.save(path)?;
        }
        Ok(value)
    }

    /// Returns false when the entry was already there.
    pub fn add(&mut self, entry: Entry) -> bool {
        if self.position(&entry).is_some() {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// Returns false when there was no such entry.
    pub fn remove(&mut self, entry: &Entry) -> bool {
        match self.position(entry) {
            Some(index) => {
                self.entries.remove(index);
                true
            }
            None => false,
        }
    }

    fn position(&self, entry: &Entry) -> Option<usize> {
        let key = |e: &Entry| {
            (
                e.text.to_lowercase(),
                e.heard.as_ref().map(|h| h.to_lowercase()),
            )
        };
        self.entries.iter().position(|e| key(e) == key(entry))
    }

    /// Every distinct spelling, for the Whisper glossary and the cleanup prompt.
    pub fn terms(&self) -> Vec<&str> {
        let mut terms: Vec<&str> = Vec::new();
        for entry in &self.entries {
            if !terms.iter().any(|t| t.eq_ignore_ascii_case(&entry.text)) {
                terms.push(&entry.text);
            }
        }
        terms
    }

    pub fn corrections(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .filter_map(|e| e.heard.as_deref().map(|heard| (heard, e.text.as_str())))
    }

    /// Replaces each heard word with its correction, matching whole words and ignoring case.
    pub fn apply(&self, text: &str) -> String {
        self.corrections()
            .fold(text.to_string(), |text, (heard, meant)| {
                replace_word(&text, heard, meant)
            })
    }
}

/// True when `word` appears in `text` as a whole word, ignoring case.
pub fn contains_word(text: &str, word: &str) -> bool {
    find_word(text, word).is_some()
}

/// The char index where `word` first appears in `text` as a whole word, ignoring case.
pub fn find_word(text: &str, word: &str) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = word.chars().collect();
    (0..chars.len()).find(|&at| word_at(&chars, at, &needle))
}

fn replace_word(text: &str, heard: &str, meant: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = heard.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if word_at(&chars, i, &needle) {
            out.push_str(meant);
            i += needle.len();
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn word_at(chars: &[char], at: usize, needle: &[char]) -> bool {
    let end = at + needle.len();
    if needle.is_empty() || end > chars.len() {
        return false;
    }
    let same = chars[at..end]
        .iter()
        .zip(needle)
        .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()));
    let boundary = |c: Option<&char>| c.is_none_or(|c| !c.is_alphanumeric());
    same && boundary(at.checked_sub(1).and_then(|i| chars.get(i))) && boundary(chars.get(end))
}

pub fn path() -> PathBuf {
    config::config_dir().join("dictionary.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_edits_keep_all_entries() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("dictionary-test-{}", std::process::id()));
        let path = dir.join("dictionary.txt");
        std::thread::scope(|scope| {
            for i in 0..12 {
                let path = &path;
                scope.spawn(move || {
                    Dictionary::edit_at(path, |d| {
                        d.add(Entry::new(&format!("term{i}"), None)?);
                        Ok(())
                    })
                    .unwrap()
                });
            }
        });
        assert_eq!(Dictionary::load_at(&path).unwrap().entries.len(), 12);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn edits_preserve_rejected_lines_comments_and_spacing() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("dictionary-round-trip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dictionary.txt");
        let rejected = format!("# my terms\r\na -> b -> c\n{} -> term\n\n", "x".repeat(101));
        let original = format!("  Pickforge  \n{rejected}Bitforge -> Pickforge\n# last comment");
        std::fs::write(&path, &original).unwrap();
        Dictionary::edit_at(&path, |dictionary| {
            assert!(dictionary.add(Entry::new("New term", None)?));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{original}\nNew term\n")
        );
        Dictionary::edit_at(&path, |dictionary| {
            assert!(dictionary.remove(&Entry::new("Pickforge", Some("Bitforge"))?));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("  Pickforge  \n{rejected}# last comment\nNew term\n")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unchanged_edits_do_not_replace_the_dictionary_file() {
        use std::os::unix::fs::MetadataExt;
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("dictionary-noop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dictionary.txt");
        let original = "# user comment\na -> b -> c\n  Pickforge  ";
        std::fs::write(&path, original).unwrap();
        let before = std::fs::metadata(&path).unwrap();
        Dictionary::edit_at(&path, |dictionary| {
            assert!(!dictionary.add(Entry::new("pickforge", None)?));
            assert!(!dictionary.remove(&Entry::new("Absent", None)?));
            Ok(())
        })
        .unwrap();
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(before.ino(), after.ino());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_terms_and_corrections_and_skips_bad_lines() {
        let dictionary = Dictionary::parse(
            "# comment\nPickforge\n  Bitforge ->  Pickforge \n -> Empty\n\nProva   Clara\n",
        );
        assert_eq!(
            dictionary.entries,
            [
                Entry::new("Pickforge", None).unwrap(),
                Entry::new("Pickforge", Some("Bitforge")).unwrap(),
                Entry::new("Prova Clara", None).unwrap(),
            ]
        );
        assert_eq!(dictionary.terms(), ["Pickforge", "Prova Clara"]);
    }

    #[test]
    fn lines_round_trip() {
        let dictionary = Dictionary::parse("Pickforge\nBitforge -> Pickforge\n");
        let text: String = dictionary.entries.iter().map(|e| e.line() + "\n").collect();
        assert_eq!(Dictionary::parse(&text), dictionary);
    }

    #[test]
    fn whitespace_is_normalized_before_rejecting_controls() {
        for whitespace in ['\t', '\n', '\r', '\u{85}'] {
            let entry = Entry::new(&format!("Pick{whitespace}forge"), Some("Bit\tforge")).unwrap();
            assert_eq!(entry.text, "Pick forge");
            assert_eq!(entry.heard.as_deref(), Some("Bit forge"));
        }
        let dictionary = Dictionary::parse("Bitforge\t->\tPickforge\n\tOther\tterm\t\n");
        assert_eq!(dictionary.entries.len(), 2);
        let lines: String = dictionary.entries.iter().map(|e| e.line() + "\n").collect();
        assert_eq!(Dictionary::parse(&lines), dictionary);
        assert_eq!(
            dictionary.entries[0],
            Entry::new("Pickforge", Some("Bitforge")).unwrap()
        );
    }

    #[test]
    fn corrections_replace_whole_words_ignoring_case() {
        let dictionary = Dictionary::parse("Bitforge -> Pickforge\npick check -> PickCheck\n");
        assert_eq!(
            dictionary.apply("O bitforge e o Pick Check; bitforged fica."),
            "O Pickforge e o PickCheck; bitforged fica."
        );
    }

    #[test]
    fn contains_word_matches_whole_words_only() {
        assert!(contains_word("O Bitforge caiu", "bitforge"));
        assert!(!contains_word("bitforged", "bitforge"));
    }

    #[test]
    fn add_and_remove_ignore_case_and_duplicates() {
        let mut dictionary = Dictionary::default();
        assert!(dictionary.add(Entry::new("Pickforge", Some("Bitforge")).unwrap()));
        assert!(!dictionary.add(Entry::new("pickforge", Some("bitforge")).unwrap()));
        assert!(dictionary.add(Entry::new("Pickforge", None).unwrap()));
        assert!(!dictionary.remove(&Entry::new("Pickforge", Some("Other")).unwrap()));
        assert!(dictionary.remove(&Entry::new("Pickforge", Some("BITFORGE")).unwrap()));
        assert_eq!(dictionary.entries, [Entry::new("Pickforge", None).unwrap()]);
    }

    #[test]
    fn invalid_entries_are_rejected() {
        for control in ['\0', '\u{7f}', '\u{1b}'] {
            let value = format!("a{control}b");
            assert!(Entry::new(&value, None).is_err());
            assert!(Entry::new("term", Some(&value)).is_err());
        }
        assert!(
            Dictionary::parse("bad\0term\nGood")
                .entries
                .iter()
                .all(|e| e.text == "Good")
        );
        assert!(Entry::new("  ", None).is_err());
        assert!(Entry::new("a -> b", None).is_err());
        assert!(Entry::new("Pickforge", Some("pickforge")).is_err());
        assert!(Entry::new(&"x".repeat(101), None).is_err());
    }
}
