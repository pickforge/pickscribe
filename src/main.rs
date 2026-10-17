mod audio;
mod cleanup;
mod config;
mod control;
mod hotkey;
mod output;
mod overlay;
mod pipeline;
mod transcribe;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::control::Command;
use crate::pipeline::Controller;
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
    /// Transcribe a 16 kHz mono WAV file and print the text.
    Transcribe {
        path: PathBuf,
        /// Also run the AI cleanup.
        #[arg(long)]
        clean: bool,
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
        Cmd::Transcribe { path, clean } => transcribe_file(&path, clean),
    }
}

fn daemon() -> Result<()> {
    let config = Config::load()?;
    let (commands, receiver) = mpsc::channel();
    control::listen(commands.clone())?;
    let transcriber = Transcriber::load(&config)?;
    hotkey::spawn(commands);
    let (ui, events) = async_channel::unbounded();
    let level = audio::Level::default();
    let controller = Controller::new(config, transcriber, ui, level.clone());
    thread::spawn(move || controller.run(receiver));
    overlay::run(events, level);
    Ok(())
}

fn transcribe_file(path: &Path, clean: bool) -> Result<()> {
    let config = Config::load()?;
    let pcm = read_wav(path)?;
    let text = Transcriber::load(&config)?.transcribe(&pcm)?;
    let text = if clean {
        let key = config::api_key()
            .context("no ANTHROPIC_API_KEY in ~/.config/pickscribe/env or the environment")?;
        cleanup::clean(&config, &key, &text)?
    } else {
        text
    };
    println!("{text}");
    Ok(())
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
