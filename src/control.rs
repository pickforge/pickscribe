use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

use anyhow::{Context, Result, bail};

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

/// Sends one command to the running daemon.
pub fn send(command: Command) -> Result<()> {
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path).with_context(|| {
        format!(
            "PickScribe is not running (no socket at {})",
            path.display()
        )
    })?;
    writeln!(stream, "{}", command.as_line())?;
    Ok(())
}

/// Binds the control socket and forwards each command it receives. Fails when another
/// daemon already owns the socket.
pub fn listen(commands: Sender<Command>) -> Result<()> {
    let path = socket_path()?;
    if UnixStream::connect(&path).is_ok() {
        bail!("PickScribe is already running");
    }
    let _ = std::fs::remove_file(&path);
    let listener =
        UnixListener::bind(&path).with_context(|| format!("cannot bind {}", path.display()))?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_ok()
                && let Some(command) = Command::parse(&line)
                && commands.send(command).is_err()
            {
                return;
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
