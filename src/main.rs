mod audio;
mod cleanup;
mod config;
mod control;
mod dictionary;
mod history;
mod hotkey;
mod mcp;
mod output;
mod overlay;
mod pipeline;
mod review;
mod status;
mod transcribe;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::control::Command;
use crate::dictionary::{Dictionary, Entry};
use crate::pipeline::Controller;
use crate::status::Hub;
use crate::transcribe::Transcriber;

#[derive(Parser)]
#[command(
    version,
    about = "Tap Right Alt, speak, tap again: the text lands in the focused app."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the dictation daemon (the default).
    Daemon,
    /// Start or stop dictation, like tapping Right Alt.
    Toggle {
        /// Skip the AI cleanup, like Shift + Right Alt.
        #[arg(long)]
        raw: bool,
    },
    /// Discard the current recording, like Esc.
    Cancel,
    /// Print the daemon's status as one JSON line.
    Status,
    /// Print the daemon's live status as JSON lines.
    Watch,
    /// List, copy, or delete recent dictations.
    History {
        #[command(subcommand)]
        action: Option<HistoryAction>,
        /// Print JSON, newest first.
        #[arg(long)]
        json: bool,
    },
    /// List or edit the dictionary of terms and corrections.
    Dict {
        #[command(subcommand)]
        action: Option<DictAction>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// List, accept, or ignore suggested dictionary corrections.
    Suggestions {
        #[command(subcommand)]
        action: Option<SuggestionAction>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Review the AI dictations not reviewed yet for misheard words, now.
    Review,
    /// Run a dictionary server for AI agents over MCP (stdio).
    Mcp,
    /// Transcribe a 16 kHz mono WAV file and print the text.
    Transcribe {
        path: PathBuf,
        /// Also run the AI cleanup.
        #[arg(long)]
        clean: bool,
    },
    /// Clean up a transcript read from stdin and print the result.
    Clean {
        /// Override cleanup_effort.
        #[arg(long)]
        effort: Option<String>,
        /// Let the model think first.
        #[arg(long)]
        thinking: bool,
    },
}

#[derive(Subcommand)]
enum HistoryAction {
    /// Copy one dictation to the clipboard.
    Copy { id: String },
    /// Delete one dictation.
    Delete { id: String },
    /// Delete every dictation.
    Clear,
}

#[derive(Subcommand)]
enum SuggestionAction {
    /// Add the suggestion to the dictionary as a correction.
    Accept { id: String },
    /// Drop the suggestion and never suggest it again.
    Ignore { id: String },
}

#[derive(Subcommand)]
enum DictAction {
    /// Add a term, or a correction with --heard.
    Add {
        text: String,
        /// What speech recognition writes instead of TEXT.
        #[arg(long)]
        heard: Option<String>,
    },
    /// Remove a term, or a correction with --heard.
    Remove {
        text: String,
        #[arg(long)]
        heard: Option<String>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("pickscribe: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command.unwrap_or(Cmd::Daemon) {
        Cmd::Daemon => daemon(),
        Cmd::Toggle { raw } => control::send(Command::Toggle { raw }),
        Cmd::Cancel => control::send(Command::Cancel),
        Cmd::Status => control::watch(true),
        Cmd::Watch => control::watch(false),
        Cmd::History { action, json } => history_command(action, json),
        Cmd::Dict { action, json } => dict_command(action, json),
        Cmd::Suggestions { action, json } => suggestions_command(action, json),
        Cmd::Review => review_now(),
        Cmd::Mcp => mcp::serve(),
        Cmd::Transcribe { path, clean } => transcribe_file(&path, clean),
        Cmd::Clean { effort, thinking } => clean_stdin(effort, thinking),
    }
}

fn daemon() -> Result<()> {
    let config = Config::load()?;
    let hub = Arc::new(Hub::default());
    let (commands, receiver) = mpsc::channel();
    control::listen(commands.clone(), Arc::clone(&hub))?;
    let transcriber = Transcriber::load(&config)?;
    hotkey::spawn(commands);
    let (ui, events) = async_channel::unbounded();
    let level = audio::Level::default();
    let controller = Controller {
        config,
        transcriber,
        ui,
        level: level.clone(),
        hub,
        client: cleanup::Client::new()?,
        reviewing: Arc::default(),
    };
    thread::spawn(move || controller.run(receiver));
    overlay::run(events, level);
    Ok(())
}

fn history_command(action: Option<HistoryAction>, json: bool) -> Result<()> {
    match action {
        None => {
            let mut items = history::load()?;
            items.reverse();
            if json {
                println!("{}", serde_json::to_string(&items)?);
            } else {
                for item in items {
                    println!("{}\t{}", item.id, item.text.replace('\n', " "));
                }
            }
            return Ok(());
        }
        Some(HistoryAction::Copy { id }) => {
            let items = history::load()?;
            let item = items
                .iter()
                .find(|item| item.id == id)
                .with_context(|| format!("no dictation with id {id}"))?;
            return output::copy(&item.text);
        }
        Some(HistoryAction::Delete { id }) => {
            let mut found = false;
            history::edit(|items| {
                let before = items.len();
                items.retain(|item| item.id != id);
                found = items.len() != before;
            })?;
            if !found {
                bail!("no dictation with id {id}");
            }
        }
        Some(HistoryAction::Clear) => history::edit(Vec::clear)?,
    }
    control::notify_history_changed();
    Ok(())
}

fn dict_command(action: Option<DictAction>, json: bool) -> Result<()> {
    let mut dictionary = Dictionary::load()?;
    match action {
        None if json => println!("{}", serde_json::to_string(&dictionary.entries)?),
        None => {
            for entry in &dictionary.entries {
                match &entry.heard {
                    Some(heard) => println!("{heard} -> {}", entry.text),
                    None => println!("{}", entry.text),
                }
            }
        }
        Some(DictAction::Add { text, heard }) => {
            dictionary.add(Entry::new(&text, heard.as_deref())?);
            dictionary.save()?;
            control::notify_dictionary_changed();
        }
        Some(DictAction::Remove { text, heard }) => {
            if !dictionary.remove(&Entry::new(&text, heard.as_deref())?) {
                bail!("no such dictionary entry");
            }
            dictionary.save()?;
            control::notify_dictionary_changed();
        }
    }
    Ok(())
}

fn suggestions_command(action: Option<SuggestionAction>, json: bool) -> Result<()> {
    match action {
        None if json => println!("{}", serde_json::to_string(&review::pending()?)?),
        None => {
            for s in review::pending()? {
                println!("{}\t{} -> {}\t{}", s.id, s.heard, s.meant, s.example);
            }
        }
        Some(SuggestionAction::Accept { id }) => {
            review::accept(&id)?;
            control::notify_suggestions_changed();
        }
        Some(SuggestionAction::Ignore { id }) => {
            review::reject(&id)?;
            control::notify_suggestions_changed();
        }
    }
    Ok(())
}

fn review_now() -> Result<()> {
    let config = Config::load()?;
    let key = config::api_key()
        .context("no ANTHROPIC_API_KEY in ~/.config/pickscribe/env or the environment")?;
    let due = review::unreviewed(&history::load()?)?;
    if due.is_empty() {
        println!("No new AI dictations to review.");
        return Ok(());
    }
    let fresh = review::review(&cleanup::Client::new()?, &config, &key, &due)?;
    control::notify_suggestions_changed();
    println!(
        "Reviewed {} dictations, {} new suggestions.",
        due.len(),
        fresh.len()
    );
    for s in fresh {
        println!("{}\t{} -> {}\t{}", s.id, s.heard, s.meant, s.reason);
    }
    Ok(())
}

fn transcribe_file(path: &Path, clean: bool) -> Result<()> {
    let config = Config::load()?;
    let dictionary = Dictionary::load()?;
    let pcm = read_wav(path)?;
    let started = Instant::now();
    let transcriber = Transcriber::load(&config)?;
    eprintln!("pickscribe: model loaded in {:?}", started.elapsed());
    let started = Instant::now();
    let heard = transcriber.transcribe(&pcm, &dictionary.terms())?;
    if !heard.doubtful.is_empty() {
        eprintln!("pickscribe: unsure about {}", heard.doubtful.join(", "));
    }
    let text = dictionary.apply(&heard.text);
    eprintln!(
        "pickscribe: {:.1} s of audio transcribed in {:?}",
        pcm.len() as f32 / audio::SAMPLE_RATE as f32,
        started.elapsed()
    );
    let text = if clean {
        clean_text(&config, &dictionary, &text)?
    } else {
        text
    };
    println!("{text}");
    Ok(())
}

fn clean_stdin(effort: Option<String>, thinking: bool) -> Result<()> {
    let mut config = Config::load()?;
    if let Some(effort) = effort {
        config.cleanup_effort = effort;
    }
    config.cleanup_thinking |= thinking;
    config.validate()?;
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let dictionary = Dictionary::load()?;
    println!("{}", clean_text(&config, &dictionary, text.trim())?);
    Ok(())
}

fn clean_text(config: &Config, dictionary: &Dictionary, text: &str) -> Result<String> {
    let key = config::api_key()
        .context("no ANTHROPIC_API_KEY in ~/.config/pickscribe/env or the environment")?;
    let started = Instant::now();
    let text = cleanup::Client::new()?.clean(config, dictionary, &key, text)?;
    eprintln!("pickscribe: cleaned in {:?}", started.elapsed());
    Ok(dictionary.apply(&text))
}

fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_rate != audio::SAMPLE_RATE || spec.channels != 1 {
        bail!("expected 16 kHz mono audio; convert with: ffmpeg -i in.wav -ar 16000 -ac 1 out.wav");
    }
    match spec.sample_format {
        hound::SampleFormat::Float => Ok(reader.samples::<f32>().collect::<Result<_, _>>()?),
        hound::SampleFormat::Int => {
            let scale = 2f32.powi(i32::from(spec.bits_per_sample) - 1);
            Ok(reader
                .samples::<i32>()
                .map(|s| s.map(|s| s as f32 / scale))
                .collect::<Result<_, _>>()?)
        }
    }
}
