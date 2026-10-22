use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::status::Hub;

/// Socket lines that are not commands for the recorder.
const WATCH: &str = "watch";
const HISTORY_CHANGED: &str = "history-changed";
const DICTIONARY_CHANGED: &str = "dictionary-changed";
const SUGGESTIONS_CHANGED: &str = "suggestions-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Starts a recording, or stops the current one and delivers its text. `raw` skips the
    /// AI cleanup, so the text never leaves the machine.
    Toggle {
        raw: bool,
    },
    Cancel,
}

impl Command {
    fn as_line(self) -> &'static str {
        match self {
            Self::Toggle { raw: false } => "toggle",
            Self::Toggle { raw: true } => "toggle-raw",
            Self::Cancel => "cancel",
        }
    }

    fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "toggle" => Some(Self::Toggle { raw: false }),
            "toggle-raw" => Some(Self::Toggle { raw: true }),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }
}

fn socket_path() -> Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    Ok(PathBuf::from(runtime).join("pickscribe.sock"))
}

fn connect() -> Result<UnixStream> {
    let path = socket_path()?;
    UnixStream::connect(&path).with_context(|| {
        format!(
            "PickScribe is not running (no socket at {})",
            path.display()
        )
    })
}

/// Sends one command to the running daemon.
pub fn send(command: Command) -> Result<()> {
    writeln!(connect()?, "{}", command.as_line())?;
    Ok(())
}

/// Tells a running daemon that a file changed, so watchers reload it. Does nothing when the
/// daemon is not running.
pub fn notify_history_changed() {
    notify(HISTORY_CHANGED);
}

pub fn notify_dictionary_changed() {
    notify(DICTIONARY_CHANGED);
}

pub fn notify_suggestions_changed() {
    notify(SUGGESTIONS_CHANGED);
}

fn notify(line: &str) {
    if let Ok(mut stream) = connect() {
        let _ = writeln!(stream, "{line}");
    }
}

/// Prints the daemon's status snapshots as JSON lines until it stops, or only the current
/// one with `once`.
pub fn watch(once: bool) -> Result<()> {
    let mut stream = connect()?;
    writeln!(stream, "{WATCH}")?;
    let mut stdout = std::io::stdout().lock();
    for line in BufReader::new(stream).lines() {
        writeln!(stdout, "{}", line?)?;
        stdout.flush()?;
        if once {
            break;
        }
    }
    Ok(())
}

/// Binds the control socket and forwards each command it receives. Fails when another
/// daemon already owns the socket.
pub fn listen(commands: Sender<Command>, hub: Arc<Hub>) -> Result<File> {
    let path = socket_path()?;
    let (listener, lock) = bind(&path)?;
    thread::spawn(move || {
        serve(listener, commands, hub);
    });
    Ok(lock)
}

fn bind(path: &Path) -> Result<(UnixListener, File)> {
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path.with_extension("lock"))?;
    lock.try_lock()
        .context("PickScribe is already running or its lock is unavailable")?;
    if UnixStream::connect(path).is_ok() {
        bail!("PickScribe is already running");
    }
    let _ = std::fs::remove_file(path);
    let listener =
        UnixListener::bind(path).with_context(|| format!("cannot bind {}", path.display()))?;
    Ok((listener, lock))
}

fn serve(listener: UnixListener, commands: Sender<Command>, hub: Arc<Hub>) {
    for stream in listener.incoming().flatten() {
        // A client that never finishes its line must not block the others.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        match line.trim() {
            WATCH => hub.watch(stream),
            HISTORY_CHANGED => hub.history_changed(),
            DICTIONARY_CHANGED => hub.dictionary_changed(),
            SUGGESTIONS_CHANGED => hub.suggestions_changed(),
            other => {
                if let Some(command) = Command::parse(other)
                    && commands.send(command).is_err()
                {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_lock_prevents_replacing_the_live_socket() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("control-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.sock");
        let (listener, lock) = bind(&path).unwrap();
        assert!(bind(&path).is_err());
        assert!(UnixStream::connect(&path).is_ok());
        drop(listener);
        drop(lock);
        let (listener, lock) = bind(&path).unwrap();
        drop(listener);
        drop(lock);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn commands_round_trip_through_their_wire_form() {
        for command in [
            Command::Toggle { raw: false },
            Command::Toggle { raw: true },
            Command::Cancel,
        ] {
            assert_eq!(Command::parse(command.as_line()), Some(command));
        }
        assert_eq!(
            Command::parse("toggle\n"),
            Some(Command::Toggle { raw: false })
        );
        assert_eq!(Command::parse("quit"), None);
    }
}
