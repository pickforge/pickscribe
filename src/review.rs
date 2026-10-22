//! Looks back over recent AI dictations for words Whisper probably misheard, and turns them
//! into suggestions the user accepts or ignores. Nothing reaches the dictionary without the
//! user's yes. Raw dictations are never reviewed, because they must not leave the machine.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cleanup;
use crate::config::{self, Config};
use crate::dictionary::{self, Dictionary, Entry};
use crate::history::{self, Item, Mode};
use crate::status::Hub;

/// At most this many suggestions per review, so a bad batch cannot flood the user.
const MAX_SUGGESTIONS: usize = 5;
const EXAMPLE_CHARS: usize = 90;
const MAX_BATCH: usize = 50;
const MANUAL_BATCH: usize = 10;

const INSTRUCTIONS: &str = "\
You review dictations for speech recognition errors. The user dictates in Brazilian Portuguese \
and English, often about software. Each dictation shows the text Whisper heard, the cleaned \
text, and the words Whisper was unsure about.
Find words that are probably misrecognized names, products, or technical terms: they sound like \
a dictionary term or a well-known name but are spelled differently, or they make no sense in \
context. Ignore ordinary words, slang, typos the cleanup already fixed, and anything you are \
not confident about. Never report a word only because it is informal.
Each `heard` is only the misheard word or words, never the words around them, and must \
appear exactly so in the heard text. `meant` is the spelling \
the user most likely said. Reply with a JSON array only, without commentary: \
[{\"heard\": \"...\", \"meant\": \"...\"}]. Reply [] when nothing \
looks wrong.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub heard: String,
    pub meant: String,
    /// The history entry that supplies context while it still exists.
    #[serde(default)]
    pub history_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pair {
    heard: String,
    meant: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    /// History id of the newest dictation already reviewed.
    #[serde(default)]
    reviewed_through: String,
    #[serde(default)]
    pending: Vec<Suggestion>,
    #[serde(default)]
    rejected: Vec<Pair>,
}

fn path() -> PathBuf {
    config::data_dir().join("suggestions.json")
}

fn read_store(path: &Path) -> Result<(Store, bool)> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_store(&text, history::load),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok((Store::default(), false)),
        Err(err) => Err(err).context("cannot read suggestions"),
    }
}

fn parse_store(text: &str, history: impl FnOnce() -> Result<Vec<Item>>) -> Result<(Store, bool)> {
    let invalid = || anyhow::anyhow!("cannot parse suggestions");
    let value: Value = serde_json::from_str(text).map_err(|_| invalid())?;
    let mut store: Store = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    let legacy = value["pending"].as_array().is_some_and(|pending| {
        pending
            .iter()
            .any(|s| s.get("example").is_some() || s.get("reason").is_some())
    });
    if legacy {
        let items = history()?;
        for (suggestion, old) in store
            .pending
            .iter_mut()
            .zip(value["pending"].as_array().unwrap())
        {
            if !suggestion.history_id.is_empty() {
                continue;
            }
            if let Some(item) = legacy_context(&items, suggestion, old) {
                suggestion.history_id.clone_from(&item.id);
            }
        }
    }
    Ok((store, legacy))
}

fn legacy_context<'a>(items: &'a [Item], suggestion: &Suggestion, old: &Value) -> Option<&'a Item> {
    let candidates = items.iter().rev().filter(|item| {
        item.mode == Mode::Ai && dictionary::contains_word(&item.transcript, &suggestion.heard)
    });
    let example = old["example"].as_str();
    let batch_id = suggestion.id.rsplit_once('-').map(|(id, _)| id);
    candidates
        .clone()
        .find(|item| match example {
            Some(example) => excerpt(&item.transcript, &suggestion.heard) == example,
            None => Some(item.id.as_str()) == batch_id,
        })
        .or_else(|| candidates.into_iter().next())
}

fn load() -> Result<Store> {
    let (store, legacy) = read_store(&path())?;
    if legacy {
        // Keep a history reference before dropping old excerpts and model explanations.
        edit(|_| Ok(()))?;
        return read_store(&path()).map(|(store, _)| store);
    }
    Ok(store)
}

/// Applies `change` under a lock, so the daemon and the CLI never overwrite each other.
fn edit<T>(change: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
    edit_at(&path(), change)
}

fn edit_at<T>(path: &Path, change: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
    std::fs::create_dir_all(path.parent().context("suggestions path has no parent")?)?;
    let lock = File::create(path.with_extension("lock"))?;
    lock.lock()?;
    let (mut store, _) = read_store(path)?;
    let value = change(&mut store)?;
    config::write_private(path, serde_json::to_string_pretty(&store)?.as_bytes())?;
    Ok(value)
}

pub fn pending() -> Result<Vec<Suggestion>> {
    Ok(load()?.pending)
}

#[derive(Serialize)]
pub struct SuggestionContext {
    #[serde(flatten)]
    pub suggestion: Suggestion,
    pub context: String,
}

pub fn pending_with_context() -> Result<Vec<SuggestionContext>> {
    let items = history::load()?;
    Ok(pending()?
        .into_iter()
        .map(|suggestion| {
            let context = suggestion_context(&suggestion, &items);
            SuggestionContext {
                suggestion,
                context,
            }
        })
        .collect())
}

fn suggestion_context(suggestion: &Suggestion, items: &[Item]) -> String {
    items
        .iter()
        .find(|item| item.id == suggestion.history_id)
        .map(|item| excerpt(&item.transcript, &suggestion.heard))
        .unwrap_or_default()
}

/// AI dictations newer than the last review, oldest first.
pub fn unreviewed(items: &[Item], limit: usize) -> Result<Vec<Item>> {
    let through = load()?.reviewed_through.parse::<u128>().unwrap_or(0);
    Ok(newer_ai(items, through, limit))
}

fn newer_ai(items: &[Item], through: u128, limit: usize) -> Vec<Item> {
    items
        .iter()
        .filter(|item| item.mode == Mode::Ai)
        .filter(|item| item.id.parse::<u128>().unwrap_or(0) > through)
        .take(limit.min(MAX_BATCH))
        .cloned()
        .collect()
}

pub fn batch_size(review_every: usize) -> usize {
    if review_every == 0 {
        MANUAL_BATCH
    } else {
        review_every.min(MAX_BATCH)
    }
}

fn due_batch(items: &[Item], through: u128, review_every: usize) -> Vec<Item> {
    let due = items
        .iter()
        .filter(|item| item.mode == Mode::Ai && item.id.parse::<u128>().unwrap_or(0) > through);
    if review_every == 0 || due.count() < review_every {
        return Vec::new();
    }
    newer_ai(items, through, batch_size(review_every))
}

/// Reviews `items` and stores what it finds. Returns only the new suggestions.
pub fn review(
    client: &cleanup::Client,
    config: &Config,
    api_key: &str,
    items: &[Item],
) -> Result<Vec<Suggestion>> {
    let Some(last) = items.last() else {
        return Ok(Vec::new());
    };
    let dictionary = Dictionary::load()?;
    review_reply(
        client.ask(api_key, &request_body(config, &dictionary, items)),
        |found| {
            edit(|store| {
                let fresh = accept_found(store, &dictionary, items, found);
                store.pending.extend(fresh.iter().cloned());
                store.reviewed_through.clone_from(&last.id);
                Ok(fresh)
            })
        },
    )
}

fn review_reply(
    reply: Result<String>,
    save: impl FnOnce(Vec<Found>) -> Result<Vec<Suggestion>>,
) -> Result<Vec<Suggestion>> {
    let found = match reply {
        Ok(reply) => parse_reply(&reply),
        Err(err) if err.is::<cleanup::UnusableReply>() => Err(err),
        Err(err) => return Err(err),
    };
    match found {
        Ok(found) => save(found),
        Err(err) => {
            save(Vec::new())?;
            Err(err)
        }
    }
}

fn request_body(config: &Config, dictionary: &Dictionary, items: &[Item]) -> Value {
    let corrections: Vec<String> = dictionary
        .corrections()
        .map(|(heard, meant)| format!("{heard} -> {meant}"))
        .collect();
    let mut user = format!(
        "<dictionary>\nTerms: {}\nKnown corrections: {}\n</dictionary>\n",
        dictionary.terms().join(", "),
        corrections.join("; ")
    );
    for item in items {
        user.push_str(&format!(
            "<dictation>\n<heard>{}</heard>\n<cleaned>{}</cleaned>\n<unsure>{}</unsure>\n</dictation>\n",
            item.transcript,
            item.text,
            item.doubtful.join(", ")
        ));
    }
    json!({
        "model": config.cleanup_model,
        "max_tokens": 4096,
        // A background judgment call: worth more effort than cleanup, without thinking time.
        "thinking": { "type": "disabled" },
        "output_config": { "effort": "high" },
        "system": INSTRUCTIONS,
        "messages": [{ "role": "user", "content": user }],
    })
}

#[derive(Deserialize)]
struct Found {
    heard: String,
    meant: String,
}

fn parse_reply(reply: &str) -> Result<Vec<Found>> {
    let (Some(start), Some(end)) = (reply.find('['), reply.rfind(']')) else {
        bail!("the review reply held no JSON array");
    };
    if start > end {
        bail!("the review reply held no JSON array");
    }
    serde_json::from_str(&reply[start..=end])
        .map_err(|_| anyhow::anyhow!("the review reply was not valid JSON"))
}

/// Keeps only findings that are real words in the dictations and new to the user.
fn accept_found(
    store: &Store,
    dictionary: &Dictionary,
    items: &[Item],
    found: Vec<Found>,
) -> Vec<Suggestion> {
    let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    let mut fresh: Vec<Suggestion> = Vec::new();
    for finding in found {
        let (heard, meant) = trim_shared_words(&finding.heard, &finding.meant);
        let Ok(entry) = Entry::new(&meant, Some(&heard)) else {
            continue;
        };
        let heard = entry.heard.clone().unwrap_or_default();
        let known = |pair: (&str, &str)| same(pair.0, &heard) && same(pair.1, &entry.text);
        if dictionary.corrections().any(known)
            || store.rejected.iter().any(|p| known((&p.heard, &p.meant)))
            || store
                .pending
                .iter()
                .chain(&fresh)
                .any(|s| known((&s.heard, &s.meant)))
        {
            continue;
        }
        let Some(item) = items
            .iter()
            .rev()
            .find(|item| dictionary::contains_word(&item.transcript, &heard))
        else {
            continue;
        };
        fresh.push(Suggestion {
            id: format!(
                "{}-{}",
                items.last().map_or("0", |i| i.id.as_str()),
                fresh.len()
            ),
            heard,
            meant: entry.text,
            history_id: item.id.clone(),
        });
        if fresh.len() == MAX_SUGGESTIONS {
            break;
        }
    }
    fresh
}

/// Drops words that `heard` and `meant` share at either end, so "o pix scribe travou" ->
/// "o PickScribe travou" becomes "pix scribe" -> "PickScribe".
fn trim_shared_words(heard: &str, meant: &str) -> (String, String) {
    let mut heard: Vec<&str> = heard.split_whitespace().collect();
    let mut meant: Vec<&str> = meant.split_whitespace().collect();
    let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    while heard.len() > 1 && meant.len() > 1 && same(heard[0], meant[0]) {
        heard.remove(0);
        meant.remove(0);
    }
    while heard.len() > 1 && meant.len() > 1 && same(heard[heard.len() - 1], meant[meant.len() - 1])
    {
        heard.pop();
        meant.pop();
    }
    (heard.join(" "), meant.join(" "))
}

/// A short piece of `text` around the first `word`.
fn excerpt(text: &str, word: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let at = dictionary::find_word(text, word).unwrap_or(0);
    let start = at.saturating_sub(EXAMPLE_CHARS / 2);
    let end = (start + EXAMPLE_CHARS).min(chars.len());
    let mut piece: String = chars[start..end]
        .iter()
        .collect::<String>()
        .trim()
        .to_string();
    if start > 0 {
        piece.insert(0, '…');
    }
    if end < chars.len() {
        piece.push('…');
    }
    piece
}

/// Reviews the unreviewed AI dictations once there are `review_every` of them, then asks the
/// user about each new suggestion. Runs on its own thread; errors only reach the log.
pub fn run_due(client: &cleanup::Client, config: &Config, hub: &Arc<Hub>) {
    if config.review_every == 0 {
        return;
    }
    let Some(key) = config::api_key() else {
        return;
    };
    let result = drain_due(
        || {
            let items = history::load()?;
            let through = load()?.reviewed_through.parse::<u128>().unwrap_or(0);
            Ok(due_batch(&items, through, config.review_every))
        },
        |due| {
            let fresh = review(client, config, &key, due)?;
            if !fresh.is_empty() {
                hub.suggestions_changed();
                for _ in fresh {
                    announce();
                }
            }
            Ok(())
        },
    );
    if let Err(err) = result {
        eprintln!("pickscribe: review failed: {err:#}");
    }
}

fn drain_due(
    mut next: impl FnMut() -> Result<Vec<Item>>,
    mut review: impl FnMut(&[Item]) -> Result<()>,
) -> Result<()> {
    loop {
        let due = next()?;
        if due.is_empty() {
            return Ok(());
        }
        review(&due)?;
    }
}

/// Tells the user about a suggestion. Clicking the notification opens the PickScribe panel on
/// Omarchy, where the user adds or ignores it; a stray click never changes the dictionary.
pub fn announce() {
    std::thread::spawn(move || {
        let clicked = Command::new("notify-send")
            .args([
                "--app-name=PickScribe",
                "--icon=audio-input-microphone-symbolic",
                "--action=default=Review",
                "--wait",
                "Dictionary suggestion",
                "Open PickScribe to review a possible correction.",
            ])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "default");
        if clicked {
            let _ = Command::new("omarchy-shell")
                .args(["-q", "pickforge.pickscribe", "open"])
                .status();
        }
    });
}

/// Adds the suggestion to the dictionary as a correction.
pub fn accept(id: &str) -> Result<Suggestion> {
    edit(|store| {
        accept_saved(store, id, |suggestion| {
            let entry = Entry::new(&suggestion.meant, Some(&suggestion.heard))?;
            Dictionary::edit(|dictionary| {
                dictionary.add(entry);
                Ok(())
            })
        })
    })
}

fn accept_saved(
    store: &mut Store,
    id: &str,
    save: impl FnOnce(&Suggestion) -> Result<()>,
) -> Result<Suggestion> {
    let suggestion = store
        .pending
        .iter()
        .find(|s| s.id == id)
        .with_context(|| format!("no suggestion with id {id}"))?;
    save(suggestion)?;
    take(store, id)
}

/// Drops the suggestion and remembers not to suggest it again.
pub fn reject(id: &str) -> Result<Suggestion> {
    edit(|store| {
        let suggestion = take(store, id)?;
        store.rejected.push(Pair {
            heard: suggestion.heard.clone(),
            meant: suggestion.meant.clone(),
        });
        Ok(suggestion)
    })
}

fn take(store: &mut Store, id: &str) -> Result<Suggestion> {
    let index = store
        .pending
        .iter()
        .position(|s| s.id == id)
        .with_context(|| format!("no suggestion with id {id}"))?;
    Ok(store.pending.remove(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, mode: Mode, transcript: &str) -> Item {
        let mut item = Item::new(mode, transcript.into(), transcript.into(), 1000);
        item.id = id.into();
        item
    }

    #[test]
    fn replies_parse_inside_stray_text() {
        let found =
            parse_reply("Here:\n[{\"heard\":\"Bitforge\",\"meant\":\"Pickforge\"}]").unwrap();
        assert_eq!(found[0].meant, "Pickforge");
        assert!(parse_reply("nothing").is_err());
    }

    #[test]
    fn malformed_arrays_never_panic() {
        for reply in [
            "][",
            "] stray [",
            "[",
            "]",
            "[invalid]",
            "[{}]",
            "[\"private words\"]",
        ] {
            let error = parse_reply(reply).err().expect("invalid reply");
            assert!(!format!("{error:#}").contains("private words"));
        }
        assert!(parse_reply("[]").unwrap().is_empty());
    }

    #[test]
    fn unusable_replies_advance_but_request_errors_retry_the_batch() {
        let items = [item("1", Mode::Ai, "Bitforge"), item("2", Mode::Ai, "text")];
        let dictionary = Dictionary::default();
        for reply in [
            Ok("no array".to_string()),
            Ok("[invalid]".to_string()),
            Err(anyhow::Error::new(cleanup::UnusableReply)),
        ] {
            let mut store = Store::default();
            let result = review_reply(reply, |found| {
                let fresh = accept_found(&store, &dictionary, &items, found);
                store.pending.extend(fresh.iter().cloned());
                store.reviewed_through = "2".into();
                Ok(fresh)
            });
            assert!(result.is_err());
            assert_eq!(store.reviewed_through, "2");
            assert!(store.pending.is_empty());
            assert!(newer_ai(&items, 2, 2).is_empty());
        }
        for error in ["transport failed", "HTTP 429"] {
            let result = review_reply(Err(anyhow::anyhow!(error)), |_| {
                panic!("request errors must not advance the review")
            });
            assert_eq!(result.unwrap_err().to_string(), error);
            assert_eq!(newer_ai(&items, 0, 2), items);
        }
    }

    #[test]
    fn automatic_review_drains_full_batches_and_stops_on_failure() {
        use std::cell::Cell;
        let items: Vec<_> = (1..=7)
            .map(|id| item(&id.to_string(), Mode::Ai, "a"))
            .collect();
        let through = Cell::new(0);
        let mut batches = Vec::new();
        drain_due(
            || Ok(due_batch(&items, through.get(), 2)),
            |batch| {
                batches.push(batch.iter().map(|item| item.id.clone()).collect::<Vec<_>>());
                through.set(batch.last().unwrap().id.parse().unwrap());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(batches, [vec!["1", "2"], vec!["3", "4"], vec!["5", "6"]]);
        assert_eq!(through.get(), 6);
        through.set(0);
        let mut attempts = 0;
        assert!(
            drain_due(
                || Ok(due_batch(&items, through.get(), 2)),
                |batch| {
                    attempts += 1;
                    if attempts == 2 {
                        bail!("request failed");
                    }
                    through.set(batch.last().unwrap().id.parse().unwrap());
                    Ok(())
                },
            )
            .is_err()
        );
        assert_eq!(attempts, 2);
        assert_eq!(through.get(), 2);
    }

    #[test]
    fn failed_dictionary_save_keeps_the_suggestion() {
        let mut store = Store {
            pending: vec![Suggestion {
                id: "1".into(),
                heard: "Bitforge".into(),
                meant: "Pickforge".into(),
                history_id: "1".into(),
            }],
            ..Store::default()
        };
        assert!(accept_saved(&mut store, "1", |_| bail!("save failed")).is_err());
        assert_eq!(store.pending.len(), 1);
        accept_saved(&mut store, "1", |_| Ok(())).unwrap();
        assert!(store.pending.is_empty());
    }

    #[test]
    fn stored_suggestions_discard_legacy_transcript_fields() {
        let store: Store = serde_json::from_value(json!({ "pending": [{
            "id": "1", "heard": "word", "meant": "term", "example": "private transcript", "reason": "private explanation"
        }]})).unwrap();
        let saved = serde_json::to_string(&store).unwrap();
        assert!(!saved.contains("private"));
        assert!(!saved.contains("example"));
        assert!(!saved.contains("reason"));
    }

    #[test]
    fn legacy_suggestions_keep_context_from_matching_history() {
        let items = vec![
            item("1", Mode::Ai, "earlier Bitforge dictation"),
            item("2", Mode::Ai, "another dictation"),
        ];
        let legacy = json!({
            "reviewed_through": "2",
            "rejected": [{"heard": "old", "meant": "term"}],
            "pending": [
                {"id": "2-0", "heard": "Bitforge", "meant": "Pickforge",
                 "example": "earlier Bitforge dictation", "reason": "private explanation"},
                {"id": "2-1", "heard": "missing", "meant": "term",
                 "example": "deleted history", "reason": "old reason"}
            ]
        });
        let (store, migrated) = parse_store(&legacy.to_string(), || Ok(items.clone())).unwrap();
        assert!(migrated);
        assert_eq!(store.pending[0].history_id, "1");
        assert_eq!(
            suggestion_context(&store.pending[0], &items),
            "earlier Bitforge dictation"
        );
        assert!(store.pending[1].history_id.is_empty());
        assert_eq!(store.reviewed_through, "2");
        assert_eq!(store.rejected.len(), 1);
        let saved = serde_json::to_string(&store).unwrap();
        assert!(!saved.contains("example"));
        assert!(!saved.contains("reason"));
        assert!(!saved.contains("dictation"));
    }

    #[test]
    fn legacy_context_falls_back_to_the_newest_ai_match() {
        let items = vec![
            item("1", Mode::Ai, "first Bitforge dictation"),
            item("2", Mode::Ai, "newest Bitforge dictation"),
            item("3", Mode::Raw, "raw Bitforge dictation"),
            item("4", Mode::Ai, "no matching word"),
        ];
        let legacy = json!({"pending": [
            {"id": "4-0", "heard": "Bitforge", "meant": "Pickforge", "example": "old shortened context"},
            {"id": "4-1", "heard": "Bitforge", "meant": "Pickforge", "reason": "legacy reason"},
            {"id": "4-2", "heard": "Bitforge", "meant": "Pickforge", "example": "first Bitforge dictation"}
        ]});
        let (store, migrated) = parse_store(&legacy.to_string(), || Ok(items.clone())).unwrap();
        assert!(migrated);
        assert_eq!(store.pending[0].history_id, "2");
        assert_eq!(store.pending[1].history_id, "2");
        assert_eq!(store.pending[2].history_id, "1");
        assert_eq!(
            suggestion_context(&store.pending[0], &items),
            "newest Bitforge dictation"
        );
    }

    #[test]
    fn malformed_suggestions_refuse_edits_without_changing_the_file() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("suggestions-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("suggestions.json");
        for text in [
            "{\"reviewed_through\":\"12\",\"rejected\":[",
            "{\"reviewed_through\":\"12\",\"rejected\":\"private text\"}",
            "null",
        ] {
            std::fs::write(&path, text).unwrap();
            let error =
                edit_at(&path, |_| -> Result<()> { panic!("edit must not run") }).unwrap_err();
            assert!(!format!("{error:#}").contains("private text"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn retries_use_the_same_oldest_bounded_batch() {
        let mut items = vec![
            item("1", Mode::Ai, "a"),
            item("2", Mode::Raw, "b"),
            item("3", Mode::Ai, "c"),
            item("4", Mode::Ai, "d"),
        ];
        let batch = newer_ai(&items, 0, 2);
        let picked: Vec<_> = batch.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(picked, ["1", "3"]);
        items.push(item("5", Mode::Ai, "e"));
        assert_eq!(newer_ai(&items, 0, 2), batch);
        let picked: Vec<_> = newer_ai(&items, 3, 2).into_iter().map(|i| i.id).collect();
        assert_eq!(picked, ["4", "5"]);
    }

    #[test]
    fn manual_review_uses_a_bounded_batch_when_automatic_review_is_off() {
        assert_eq!(batch_size(0), 10);
        assert_eq!(batch_size(3), 3);
        assert_eq!(batch_size(usize::MAX), MAX_BATCH);
        let items: Vec<_> = (1..=100)
            .map(|id| item(&id.to_string(), Mode::Ai, "a"))
            .collect();
        assert_eq!(newer_ai(&items, 0, batch_size(0)).len(), 10);
        let batch = newer_ai(&items, 0, usize::MAX);
        assert_eq!(batch.len(), MAX_BATCH);
        assert_eq!(batch.first().unwrap().id, "1");
        assert_eq!(batch.last().unwrap().id, MAX_BATCH.to_string());
    }

    #[test]
    fn automatic_review_keeps_its_threshold_and_bounds_each_attempt() {
        let mut items: Vec<_> = (1..=99)
            .map(|id| item(&id.to_string(), Mode::Ai, "a"))
            .collect();
        assert!(due_batch(&items, 0, 100).is_empty());
        items.push(item("100", Mode::Raw, "b"));
        assert!(due_batch(&items, 0, 100).is_empty());
        items.push(item("101", Mode::Ai, "c"));
        let batch = due_batch(&items, 0, 100);
        assert_eq!(batch.len(), MAX_BATCH);
        items.push(item("102", Mode::Ai, "d"));
        assert_eq!(due_batch(&items, 0, 100), batch);
        assert!(due_batch(&items, 0, 0).is_empty());
        assert_eq!(due_batch(&items, 50, 10)[0].id, "51");
    }

    #[test]
    fn findings_must_be_in_the_text_and_new() {
        let store = Store {
            rejected: vec![Pair {
                heard: "Candeia".into(),
                meant: "Candeeiro".into(),
            }],
            ..Store::default()
        };
        let dictionary = Dictionary::default();
        let items = [item("1", Mode::Ai, "o pix scribe e a Candeia")];
        let found = vec![
            Found {
                heard: "pix scribe".into(),
                meant: "PickScribe".into(),
            },
            Found {
                heard: "Candeia".into(),
                meant: "Candeeiro".into(),
            },
            Found {
                heard: "Bitforge".into(),
                meant: "Pickforge".into(),
            },
            Found {
                heard: "pix scribe".into(),
                meant: "PickScribe".into(),
            },
        ];
        let fresh = accept_found(&store, &dictionary, &items, found);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].heard, "pix scribe");
        assert_eq!(fresh[0].history_id, "1");
        assert_eq!(
            suggestion_context(&fresh[0], &items),
            "o pix scribe e a Candeia"
        );
        assert_eq!(suggestion_context(&fresh[0], &[]), "");
    }

    #[test]
    fn only_new_ai_dictations_are_reviewed() {
        let items = [
            item("5", Mode::Ai, "a"),
            item("6", Mode::Raw, "b"),
            item("7", Mode::Ai, "c"),
        ];
        let picked: Vec<String> = newer_ai(&items, 5, 10).into_iter().map(|i| i.id).collect();
        assert_eq!(picked, ["7"]);
    }

    #[test]
    fn shared_context_words_are_trimmed() {
        assert_eq!(
            trim_shared_words("o pix scribe travou", "o PickScribe travou"),
            ("pix scribe".to_string(), "PickScribe".to_string())
        );
        assert_eq!(
            trim_shared_words("pede pro astro", "pede pro Astra"),
            ("astro".to_string(), "Astra".to_string())
        );
        assert_eq!(
            trim_shared_words("Bitforge", "Pickforge"),
            ("Bitforge".to_string(), "Pickforge".to_string())
        );
    }

    #[test]
    fn excerpts_stay_short_around_the_word() {
        let text = format!("{} Bitforge {}", "a".repeat(100), "b".repeat(100));
        let piece = excerpt(&text, "bitforge");
        assert!(piece.contains("Bitforge"));
        assert!(piece.starts_with('…') && piece.ends_with('…'));
        assert!(piece.chars().count() <= EXAMPLE_CHARS + 2);
    }
}
