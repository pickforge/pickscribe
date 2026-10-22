//! Looks back over recent AI dictations for words Whisper probably misheard, and turns them
//! into suggestions the user accepts or ignores. Nothing reaches the dictionary without the
//! user's yes. Raw dictations are never reviewed, because they must not leave the machine.

use std::fs::File;
use std::path::PathBuf;
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

fn read_store() -> Result<Store> {
    match std::fs::read_to_string(path()) {
        Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(err) => Err(err).context("cannot read suggestions"),
    }
}

fn load() -> Result<Store> {
    // Rewriting old stores drops the former transcript excerpts and model explanations.
    let text = match std::fs::read_to_string(path()) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(err) => return Err(err).context("cannot read suggestions"),
    };
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if value["pending"].as_array().is_some_and(|pending| {
        pending
            .iter()
            .any(|s| s.get("example").is_some() || s.get("reason").is_some())
    }) {
        edit(|_| Ok(()))?;
        return read_store();
    }
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Applies `change` under a lock, so the daemon and the CLI never overwrite each other.
fn edit<T>(change: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
    let dir = config::data_dir();
    std::fs::create_dir_all(&dir)?;
    let lock = File::create(dir.join("suggestions.lock"))?;
    lock.lock()?;
    let mut store = read_store()?;
    let value = change(&mut store)?;
    config::write_private(&path(), serde_json::to_string_pretty(&store)?.as_bytes())?;
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
    let mut due: Vec<Item> = items
        .iter()
        .rev()
        .filter(|item| item.mode == Mode::Ai)
        .filter(|item| item.id.parse::<u128>().unwrap_or(0) > through)
        .take(limit)
        .cloned()
        .collect();
    due.reverse();
    due
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
    let reply = client.ask(api_key, &request_body(config, &dictionary, items))?;
    let found = parse_reply(&reply)?;
    edit(|store| {
        let fresh = accept_found(store, &dictionary, items, found);
        store.pending.extend(fresh.iter().cloned());
        store.reviewed_through.clone_from(&last.id);
        Ok(fresh)
    })
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
    let Some(key) = config::api_key() else {
        return;
    };
    let due = match history::load().and_then(|items| unreviewed(&items, config.review_every)) {
        Ok(due) if due.len() >= config.review_every => due,
        Ok(_) => return,
        Err(err) => {
            eprintln!("pickscribe: review skipped: {err:#}");
            return;
        }
    };
    match review(client, config, &key, &due) {
        Ok(fresh) if fresh.is_empty() => {}
        Ok(fresh) => {
            hub.suggestions_changed();
            for _ in fresh {
                announce();
            }
        }
        // The dictations stay unreviewed, so the next one retries.
        Err(err) => eprintln!("pickscribe: review failed: {err:#}"),
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
    fn retries_only_use_the_newest_due_items() {
        let items = [
            item("1", Mode::Ai, "a"),
            item("2", Mode::Raw, "b"),
            item("3", Mode::Ai, "c"),
            item("4", Mode::Ai, "d"),
        ];
        let picked: Vec<_> = newer_ai(&items, 0, 2).into_iter().map(|i| i.id).collect();
        assert_eq!(picked, ["3", "4"]);
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
