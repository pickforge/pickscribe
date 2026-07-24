use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::config::PasteConfig;

use super::find_command;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryMethod {
    Auto,
    Hotkey,
    Type,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasteChord {
    CtrlV,
    CtrlShiftV,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypeBackend {
    Auto,
    Ydotool,
    Wtype,
    Xdotool,
    Osascript,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryConfig {
    pub method: DeliveryMethod,
    pub chord: PasteChord,
    pub delay_ms: u64,
    pub copy_to_clipboard: bool,
    pub type_backend: TypeBackend,
}

impl From<&PasteConfig> for DeliveryConfig {
    fn from(config: &PasteConfig) -> Self {
        Self {
            method: match config.method.as_str() {
                "hotkey" => DeliveryMethod::Hotkey,
                "type" => DeliveryMethod::Type,
                "none" => DeliveryMethod::None,
                _ => DeliveryMethod::Auto,
            },
            chord: if config.chord == "ctrl-shift-v" {
                PasteChord::CtrlShiftV
            } else {
                PasteChord::CtrlV
            },
            delay_ms: config.delay_ms,
            copy_to_clipboard: config.copy_to_clipboard,
            type_backend: TypeBackend::Auto,
        }
    }
}

#[derive(Debug, Default)]
pub struct DeliveryOutcome {
    pub clipboard_error: Option<anyhow::Error>,
    pub insertion_error: Option<anyhow::Error>,
}

impl DeliveryOutcome {
    pub fn into_result(self) -> Result<()> {
        match (self.clipboard_error, self.insertion_error) {
            (None, None) => Ok(()),
            (Some(copy), None) => Err(anyhow!("clipboard copy failed: {copy:#}")),
            (None, Some(insert)) => Err(anyhow!(
                "inserting text into active window failed: {insert:#}"
            )),
            (Some(copy), Some(insert)) => Err(anyhow!(
                "clipboard copy failed: {copy:#}; inserting text into active window failed: {insert:#}"
            )),
        }
    }
}

pub fn copy_to_clipboard(text: &str) -> Result<()> {
    ProcessRuntime.copy_to_clipboard(text)
}

pub fn deliver(config: &DeliveryConfig, text: &str) -> DeliveryOutcome {
    deliver_with(&mut ProcessRuntime, config, text)
}

fn deliver_with(
    runtime: &mut impl DeliveryRuntime,
    config: &DeliveryConfig,
    text: &str,
) -> DeliveryOutcome {
    let method = effective_method(runtime, config);
    let should_copy = config.copy_to_clipboard || method == DeliveryMethod::Hotkey;
    let clipboard_error = should_copy
        .then(|| runtime.copy_to_clipboard(text).err())
        .flatten();

    let insertion_error = if method == DeliveryMethod::Hotkey && clipboard_error.is_some() {
        None
    } else {
        match method {
            DeliveryMethod::None => None,
            DeliveryMethod::Hotkey => {
                runtime.sleep(config.delay_ms);
                let backend = hotkey_backend(runtime, config.type_backend);
                runtime.paste_with_hotkey(backend, config.chord).err()
            }
            DeliveryMethod::Type => {
                runtime.sleep(config.delay_ms);
                let backend = typing_backend(runtime, config.type_backend);
                runtime.type_text(backend, text).err()
            }
            DeliveryMethod::Auto => unreachable!(),
        }
    };

    DeliveryOutcome {
        clipboard_error,
        insertion_error,
    }
}

fn effective_method(runtime: &impl DeliveryRuntime, config: &DeliveryConfig) -> DeliveryMethod {
    match config.method {
        DeliveryMethod::Auto => {
            if auto_hotkey_backend(runtime, config.type_backend).is_some() {
                DeliveryMethod::Hotkey
            } else {
                DeliveryMethod::Type
            }
        }
        method => method,
    }
}

fn auto_hotkey_backend(
    runtime: &impl DeliveryRuntime,
    requested: TypeBackend,
) -> Option<TypeBackend> {
    if runtime.is_macos() {
        return (requested != TypeBackend::None).then_some(TypeBackend::Osascript);
    }
    match requested {
        TypeBackend::Auto => [TypeBackend::Ydotool, TypeBackend::Xdotool]
            .into_iter()
            .find(|backend| runtime.command_exists(backend.command_name())),
        TypeBackend::Ydotool | TypeBackend::Xdotool => Some(requested),
        TypeBackend::Wtype | TypeBackend::Osascript | TypeBackend::None => None,
    }
}

fn hotkey_backend(runtime: &impl DeliveryRuntime, requested: TypeBackend) -> Option<TypeBackend> {
    if runtime.is_macos() {
        return (requested != TypeBackend::None).then_some(TypeBackend::Osascript);
    }
    match requested {
        TypeBackend::Auto => [TypeBackend::Ydotool, TypeBackend::Xdotool]
            .into_iter()
            .find(|backend| runtime.command_exists(backend.command_name())),
        TypeBackend::None => None,
        backend => Some(backend),
    }
}

fn typing_backend(runtime: &impl DeliveryRuntime, requested: TypeBackend) -> Option<TypeBackend> {
    if runtime.is_macos() {
        return (requested != TypeBackend::None).then_some(TypeBackend::Osascript);
    }
    if requested != TypeBackend::Auto {
        return (requested != TypeBackend::None).then_some(requested);
    }

    let candidates = if runtime.is_wayland() {
        [
            TypeBackend::Ydotool,
            TypeBackend::Wtype,
            TypeBackend::Xdotool,
        ]
    } else {
        [
            TypeBackend::Xdotool,
            TypeBackend::Ydotool,
            TypeBackend::Wtype,
        ]
    };
    candidates
        .into_iter()
        .find(|backend| runtime.command_exists(backend.command_name()))
}

impl TypeBackend {
    fn command_name(self) -> &'static str {
        match self {
            Self::Ydotool => "ydotool",
            Self::Wtype => "wtype",
            Self::Xdotool => "xdotool",
            Self::Osascript => "osascript",
            Self::Auto | Self::None => "",
        }
    }
}

trait DeliveryRuntime {
    fn command_exists(&self, program: &str) -> bool;
    fn is_wayland(&self) -> bool;
    fn is_macos(&self) -> bool;
    fn copy_to_clipboard(&mut self, text: &str) -> Result<()>;
    fn paste_with_hotkey(&mut self, backend: Option<TypeBackend>, chord: PasteChord) -> Result<()>;
    fn type_text(&mut self, backend: Option<TypeBackend>, text: &str) -> Result<()>;
    fn sleep(&mut self, delay_ms: u64);
}

struct ProcessRuntime;

impl DeliveryRuntime for ProcessRuntime {
    fn command_exists(&self, program: &str) -> bool {
        find_command(program).is_some()
    }

    fn is_wayland(&self) -> bool {
        std::env::var("XDG_SESSION_TYPE")
            .map(|session| session.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
    }

    fn is_macos(&self) -> bool {
        cfg!(target_os = "macos")
    }

    fn copy_to_clipboard(&mut self, text: &str) -> Result<()> {
        if self.is_macos() {
            return run_with_stdin(resolved_program("pbcopy"), "pbcopy", &[], text);
        }
        if let Some(program) = find_command("wl-copy") {
            run_with_stdin(&program, "wl-copy", &[], text)
        } else if let Some(program) = find_command("xclip") {
            run_with_stdin(&program, "xclip", &["-selection", "clipboard"], text)
        } else if let Some(program) = find_command("xsel") {
            run_with_stdin(&program, "xsel", &["--clipboard", "--input"], text)
        } else {
            Err(anyhow!(
                "no clipboard helper found; install wl-clipboard, xclip, or xsel"
            ))
        }
    }

    fn paste_with_hotkey(&mut self, backend: Option<TypeBackend>, chord: PasteChord) -> Result<()> {
        match backend {
            Some(TypeBackend::Ydotool) => {
                let chord: &[&str] = match chord {
                    PasteChord::CtrlV => &[
                        "key", "29:0", "97:0", "42:0", "54:0", "56:0", "100:0", "125:0", "126:0",
                        "29:1", "47:1", "47:0", "29:0",
                    ],
                    PasteChord::CtrlShiftV => &[
                        "key", "29:0", "97:0", "42:0", "54:0", "56:0", "100:0", "125:0", "126:0",
                        "29:1", "42:1", "47:1", "47:0", "42:0", "29:0",
                    ],
                };
                run_status(resolved_program("ydotool"), "ydotool", chord)
            }
            Some(TypeBackend::Xdotool) => {
                let chord = match chord {
                    PasteChord::CtrlV => "ctrl+v",
                    PasteChord::CtrlShiftV => "ctrl+shift+v",
                };
                run_status(
                    resolved_program("xdotool"),
                    "xdotool",
                    &["key", "--clearmodifiers", chord],
                )
            }
            Some(TypeBackend::Wtype) => Err(anyhow!(
                "wtype cannot send paste shortcuts; use --paste-method type or install ydotool"
            )),
            Some(TypeBackend::Osascript) => run_osascript(osascript_chord_script(chord)),
            Some(TypeBackend::Auto | TypeBackend::None) | None => Err(anyhow!(
                "no paste hotkey backend found; install ydotool for Wayland or xdotool for X11"
            )),
        }
    }

    fn type_text(&mut self, backend: Option<TypeBackend>, text: &str) -> Result<()> {
        match backend {
            Some(TypeBackend::Ydotool) => run_with_stdin(
                resolved_program("ydotool"),
                "ydotool",
                &["type", "--file", "-"],
                text,
            ),
            Some(TypeBackend::Xdotool) => run_with_stdin(
                resolved_program("xdotool"),
                "xdotool",
                &["type", "--clearmodifiers", "--file", "-"],
                text,
            ),
            Some(TypeBackend::Wtype) => {
                run_status(resolved_program("wtype"), "wtype", &[text])
            }
            Some(TypeBackend::Osascript) => run_osascript(&osascript_type_script(text)),
            Some(TypeBackend::Auto | TypeBackend::None) | None => Err(anyhow!(
                "no typing backend found; install ydotool for Wayland, wtype if your compositor supports it, or xdotool for X11"
            )),
        }
    }

    fn sleep(&mut self, delay_ms: u64) {
        if delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }
}

fn resolved_program(name: &str) -> std::path::PathBuf {
    find_command(name).unwrap_or_else(|| name.into())
}

fn run_with_stdin(
    program: impl AsRef<std::ffi::OsStr>,
    name: &str,
    args: &[&str],
    input: &str,
) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {name}"))?;

    child
        .stdin
        .take()
        .context("failed to open child stdin")?
        .write_all(input.as_bytes())
        .with_context(|| format!("failed to write to {name} stdin"))?;

    let output = child
        .wait_with_output()
        .with_context(|| format!("failed to wait for {name}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "{name} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn run_status(
    program: impl AsRef<std::ffi::OsStr>,
    name: &str,
    args: &[&str],
) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to start {name}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "{name} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// AppleScript one-liner that synthesizes the paste chord via System Events.
/// `PasteChord::CtrlV` maps to Command-V; `PasteChord::CtrlShiftV` to
/// Command-Shift-V, matching macOS's paste and paste-and-match-style
/// shortcuts respectively.
fn osascript_chord_script(chord: PasteChord) -> &'static str {
    match chord {
        PasteChord::CtrlV => {
            r#"tell application "System Events" to keystroke "v" using command down"#
        }
        PasteChord::CtrlShiftV => {
            r#"tell application "System Events" to keystroke "v" using {command down, shift down}"#
        }
    }
}

const ACCESSIBILITY_DENIED_MESSAGE: &str = "PickScribe is not allowed to send keystrokes. Grant Accessibility access to PickScribe in System Settings \u{2192} Privacy & Security \u{2192} Accessibility, then try again.";

/// Runs an AppleScript one-liner via `osascript -e`, mapping the
/// Accessibility-denied failure (macOS error 1002, "not allowed to send
/// keystrokes") to a clear, actionable message instead of the raw
/// System Events error text.
fn run_osascript(script: &str) -> Result<()> {
    let output = Command::new(resolved_program("osascript"))
        .args(["-e", script])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| "failed to start osascript".to_string())?;

    map_osascript_output(output.status.success(), output.status.to_string(), &output.stderr)
}

/// Pure mapping from an `osascript` exit outcome to a `Result`, split out
/// from `run_osascript` so the Accessibility-denied error mapping is
/// testable without spawning a real process.
fn map_osascript_output(success: bool, status: String, stderr: &[u8]) -> Result<()> {
    if success {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(stderr);
    if stderr.contains("not allowed to send keystrokes") || stderr.contains("1002") {
        Err(anyhow!(ACCESSIBILITY_DENIED_MESSAGE))
    } else {
        Err(anyhow!(
            "osascript exited with {status}: {}",
            stderr.trim()
        ))
    }
}

/// Builds the AppleScript that types `text` via System Events keystroke
/// calls. Backslashes and double quotes are escaped so the text is safe to
/// embed in AppleScript string literals; AppleScript string literals cannot
/// contain a literal newline, so each line is sent as its own `keystroke`
/// call with a `keystroke return` in between to reproduce line breaks.
fn osascript_type_script(text: &str) -> String {
    let mut script = String::from("tell application \"System Events\"\n");
    let lines: Vec<&str> = text.split('\n').collect();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            script.push_str("\tkeystroke return\n");
        }
        script.push_str("\tkeystroke \"");
        script.push_str(&escape_applescript_string(line));
        script.push_str("\"\n");
    }
    script.push_str("end tell");
    script
}

fn escape_applescript_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeRuntime {
        available: Vec<TypeBackend>,
        wayland: bool,
        macos: bool,
        copy_fails: bool,
        insertion_fails: bool,
        copied: bool,
        pasted: Option<(TypeBackend, PasteChord)>,
        typed: Option<(TypeBackend, String)>,
        slept_ms: Vec<u64>,
    }

    impl DeliveryRuntime for FakeRuntime {
        fn command_exists(&self, program: &str) -> bool {
            self.available
                .iter()
                .any(|backend| backend.command_name() == program)
        }

        fn is_wayland(&self) -> bool {
            self.wayland
        }

        fn is_macos(&self) -> bool {
            self.macos
        }

        fn copy_to_clipboard(&mut self, _text: &str) -> Result<()> {
            self.copied = true;
            if self.copy_fails {
                Err(anyhow!("copy failed"))
            } else {
                Ok(())
            }
        }

        fn paste_with_hotkey(
            &mut self,
            backend: Option<TypeBackend>,
            chord: PasteChord,
        ) -> Result<()> {
            if let Some(backend) = backend {
                self.pasted = Some((backend, chord));
            }
            if self.insertion_fails {
                Err(anyhow!("paste failed"))
            } else {
                Ok(())
            }
        }

        fn type_text(&mut self, backend: Option<TypeBackend>, text: &str) -> Result<()> {
            if let Some(backend) = backend {
                self.typed = Some((backend, text.into()));
            }
            if self.insertion_fails {
                Err(anyhow!("type failed"))
            } else {
                Ok(())
            }
        }

        fn sleep(&mut self, delay_ms: u64) {
            self.slept_ms.push(delay_ms);
        }
    }

    fn config(method: DeliveryMethod) -> DeliveryConfig {
        DeliveryConfig {
            method,
            chord: PasteChord::CtrlV,
            delay_ms: 150,
            copy_to_clipboard: true,
            type_backend: TypeBackend::Auto,
        }
    }

    #[test]
    fn hotkey_requires_clipboard_even_when_copy_is_disabled() {
        let mut runtime = FakeRuntime {
            available: vec![TypeBackend::Ydotool],
            ..Default::default()
        };
        let mut config = config(DeliveryMethod::Hotkey);
        config.copy_to_clipboard = false;

        let outcome = deliver_with(&mut runtime, &config, "hello");

        assert!(outcome.into_result().is_ok());
        assert!(runtime.copied);
        assert_eq!(
            runtime.pasted,
            Some((TypeBackend::Ydotool, PasteChord::CtrlV))
        );
    }

    #[test]
    fn failed_clipboard_copy_does_not_send_hotkey() {
        let mut runtime = FakeRuntime {
            available: vec![TypeBackend::Ydotool],
            copy_fails: true,
            ..Default::default()
        };
        let config = config(DeliveryMethod::Hotkey);

        let outcome = deliver_with(&mut runtime, &config, "hello");

        assert_eq!(
            outcome.clipboard_error.unwrap().to_string(),
            "copy failed"
        );
        assert!(outcome.insertion_error.is_none());
        assert!(runtime.pasted.is_none());
        assert!(runtime.slept_ms.is_empty());
    }

    #[test]
    fn auto_falls_back_to_wayland_typing_backend_without_hotkey_backend() {
        let mut runtime = FakeRuntime {
            available: vec![TypeBackend::Wtype],
            wayland: true,
            ..Default::default()
        };
        let config = config(DeliveryMethod::Auto);

        let outcome = deliver_with(&mut runtime, &config, "hello");

        assert!(outcome.into_result().is_ok());
        assert_eq!(runtime.typed, Some((TypeBackend::Wtype, "hello".into())));
        assert!(runtime.pasted.is_none());
    }

    #[test]
    fn selected_type_backend_and_terminal_chord_are_preserved() {
        let mut runtime = FakeRuntime::default();
        let config = DeliveryConfig {
            method: DeliveryMethod::Hotkey,
            chord: PasteChord::CtrlShiftV,
            delay_ms: 25,
            copy_to_clipboard: false,
            type_backend: TypeBackend::Xdotool,
        };

        let outcome = deliver_with(&mut runtime, &config, "hello");

        assert!(outcome.into_result().is_ok());
        assert_eq!(
            runtime.pasted,
            Some((TypeBackend::Xdotool, PasteChord::CtrlShiftV))
        );
        assert_eq!(runtime.slept_ms, vec![25]);
    }

    #[test]
    fn successful_copy_and_failed_paste_are_reported_separately() {
        let mut runtime = FakeRuntime {
            available: vec![TypeBackend::Ydotool],
            insertion_fails: true,
            ..Default::default()
        };
        let config = config(DeliveryMethod::Hotkey);

        let outcome = deliver_with(&mut runtime, &config, "hello");

        assert!(outcome.clipboard_error.is_none());
        assert_eq!(outcome.insertion_error.unwrap().to_string(), "paste failed");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "temporary live smoke check, run manually via --ignored on macOS"]
    fn live_smoke_pbcopy_round_trips_through_process_runtime() {
        let marker = "pickscribe-macos-smoke-check";
        ProcessRuntime.copy_to_clipboard(marker).unwrap();
        let output = Command::new("pbpaste").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), marker);
    }

    #[test]
    fn no_delivery_method_can_still_copy() {
        let mut runtime = FakeRuntime::default();
        let config = config(DeliveryMethod::None);

        let outcome = deliver_with(&mut runtime, &config, "hello");

        assert!(outcome.into_result().is_ok());
        assert!(runtime.copied);
        assert!(runtime.pasted.is_none());
        assert!(runtime.typed.is_none());
        assert!(runtime.slept_ms.is_empty());
    }

    #[test]
    fn macos_auto_resolves_to_hotkey_via_osascript_without_any_linux_tool_present() {
        let mut runtime = FakeRuntime {
            macos: true,
            ..Default::default()
        };
        let config = config(DeliveryMethod::Auto);

        let outcome = deliver_with(&mut runtime, &config, "hello");

        assert!(outcome.into_result().is_ok());
        assert_eq!(
            runtime.pasted,
            Some((TypeBackend::Osascript, PasteChord::CtrlV))
        );
        assert!(runtime.typed.is_none());
    }

    #[test]
    fn macos_type_method_always_selects_osascript() {
        let mut runtime = FakeRuntime {
            macos: true,
            ..Default::default()
        };
        let config = config(DeliveryMethod::Type);

        let outcome = deliver_with(&mut runtime, &config, "hello");

        assert!(outcome.into_result().is_ok());
        assert_eq!(runtime.typed, Some((TypeBackend::Osascript, "hello".into())));
    }

    #[test]
    fn macos_none_backend_still_disables_insertion() {
        let mut runtime = FakeRuntime {
            macos: true,
            ..Default::default()
        };
        let mut config = config(DeliveryMethod::Auto);
        config.type_backend = TypeBackend::None;

        let outcome = deliver_with(&mut runtime, &config, "hello");

        // Auto with no usable backend falls back to Type, which then finds
        // no backend either, so insertion silently no-ops instead of erroring.
        assert!(outcome.into_result().is_ok());
        assert!(runtime.pasted.is_none());
        assert!(runtime.typed.is_none());
    }

    #[test]
    fn macos_chord_mapping_uses_command_and_command_shift() {
        let ctrl_v = osascript_chord_script(PasteChord::CtrlV);
        let ctrl_shift_v = osascript_chord_script(PasteChord::CtrlShiftV);

        assert!(ctrl_v.contains("using command down"));
        assert!(!ctrl_v.contains("shift"));
        assert!(ctrl_shift_v.contains("using {command down, shift down}"));
    }

    #[test]
    fn osascript_type_script_escapes_quotes_and_backslashes() {
        let script = osascript_type_script(r#"say "hi" \ bye"#);

        assert_eq!(
            script,
            "tell application \"System Events\"\n\tkeystroke \"say \\\"hi\\\" \\\\ bye\"\nend tell"
        );
    }

    #[test]
    fn osascript_type_script_sends_return_between_lines() {
        let script = osascript_type_script("line one\nline two");

        assert_eq!(
            script,
            "tell application \"System Events\"\n\tkeystroke \"line one\"\n\tkeystroke return\n\tkeystroke \"line two\"\nend tell"
        );
    }

    #[test]
    fn accessibility_denied_stderr_maps_to_actionable_message() {
        let result = map_osascript_output(
            false,
            "exit status: 1".into(),
            b"osascript: error: System Events got an error: PickScribe is not allowed to send keystrokes. (1002)",
        );

        let message = result.unwrap_err().to_string();
        assert!(message.contains("Accessibility"));
        assert!(message.contains("System Settings"));
    }

    #[test]
    fn other_osascript_failures_keep_the_raw_stderr() {
        let result = map_osascript_output(false, "exit status: 1".into(), b"some other failure");

        assert_eq!(
            result.unwrap_err().to_string(),
            "osascript exited with exit status: 1: some other failure"
        );
    }
}
