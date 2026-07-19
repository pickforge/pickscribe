use anyhow::{Context, Result, anyhow};
use clap::{Parser, ValueEnum};
use pickscribe::engine::cleanup::{
    self as cleanup_engine, CleanupCredentials, CleanupKind, CleanupPolicy, FailurePolicy,
};
use std::{
    env,
    io::{self, IsTerminal, Read, Write},
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Debug, Parser)]
#[command(
    name = "pickscribe-cleanup",
    about = "PickScribe cleanup helper: AI polish + clipboard/paste for Linux dictation"
)]
struct Args {
    /// Text to clean. If omitted, text is read from stdin.
    #[arg(value_name = "TEXT", trailing_var_arg = true)]
    text: Vec<String>,

    /// LLM provider. auto = DeepSeek if DEEPSEEK_API_KEY exists, OpenAI if OPENAI_API_KEY exists, otherwise Ollama.
    #[arg(long, value_enum, default_value = "auto", env = "PICKSCRIBE_PROVIDER")]
    provider: Provider,

    /// Model name. Defaults: deepseek-v4-flash, gpt-4o-mini, or qwen2.5:14b for Ollama.
    #[arg(long, env = "PICKSCRIBE_MODEL")]
    model: Option<String>,

    /// DeepSeek thinking mode. Disabled is best for low-latency dictation cleanup.
    #[arg(
        long,
        value_enum,
        default_value = "disabled",
        env = "PICKSCRIBE_DEEPSEEK_THINKING"
    )]
    deepseek_thinking: DeepseekThinking,

    /// Chat completions endpoint. Supports OpenAI-compatible APIs.
    #[arg(long, env = "PICKSCRIBE_ENDPOINT")]
    endpoint: Option<String>,

    /// API key. If omitted, provider-specific env vars are used.
    #[arg(long, env = "PICKSCRIBE_API_KEY")]
    api_key: Option<String>,

    /// HTTP timeout for cleanup requests.
    #[arg(long, default_value_t = 30, env = "PICKSCRIBE_TIMEOUT_SECS")]
    timeout_secs: u64,

    /// Temperature for the cleanup model.
    #[arg(long, default_value_t = 0.2, env = "PICKSCRIBE_TEMPERATURE")]
    temperature: f32,

    /// Disable LLM cleanup and use the original text.
    #[arg(long)]
    no_llm: bool,

    /// Do not copy the final text to the clipboard.
    #[arg(long)]
    no_copy: bool,

    /// Do not type/paste the final text into the active window.
    #[arg(long)]
    no_paste: bool,

    /// How to insert text into the focused app. hotkey copies then sends Ctrl+V; type simulates every character.
    #[arg(
        long,
        value_enum,
        default_value = "auto",
        env = "PICKSCRIBE_PASTE_METHOD"
    )]
    paste_method: PasteMethod,

    /// Paste key chord for hotkey paste. Use ctrl-shift-v for terminals.
    #[arg(
        long,
        value_enum,
        default_value = "ctrl-v",
        env = "PICKSCRIBE_PASTE_CHORD"
    )]
    paste_chord: PasteChord,

    /// Delay before typing/pasting so shortcut modifier keys can be released.
    #[arg(long, default_value_t = 150, env = "PICKSCRIBE_PASTE_DELAY_MS")]
    paste_delay_ms: u64,

    /// Print the final text to stdout.
    #[arg(long)]
    print: bool,

    /// Print to stdout only; implies --no-copy and --no-paste.
    #[arg(long)]
    stdout_only: bool,

    /// Exit with an error if the LLM request fails. By default, the original text is used as fallback.
    #[arg(long)]
    strict: bool,

    /// Typing backend for paste/type step.
    #[arg(
        long,
        value_enum,
        default_value = "auto",
        env = "PICKSCRIBE_TYPE_BACKEND"
    )]
    type_backend: TypeBackend,

    /// User instructions sent to the LLM before the dictated text.
    #[arg(long, env = "PICKSCRIBE_INSTRUCTIONS")]
    instructions: Option<String>,

    /// Suppress non-fatal warnings.
    #[arg(long)]
    quiet: bool,

    /// Restrict cleanup to loopback endpoints. Used by incremental segment cleanup.
    #[arg(long, env = "PICKSCRIBE_LOCAL_ONLY", hide = true)]
    local_only: bool,

    /// Use conservative prompt and output validation for incremental segment cleanup.
    #[arg(long, hide = true)]
    segment: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum Provider {
    Auto,
    Deepseek,
    Ollama,
    Openai,
    None,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum TypeBackend {
    Auto,
    Ydotool,
    Wtype,
    Xdotool,
    None,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum PasteMethod {
    Auto,
    /// Copy to clipboard and send a paste key chord to the focused app.
    Hotkey,
    /// Simulate typing every character.
    Type,
    None,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum PasteChord {
    /// Standard paste for most GUI applications.
    CtrlV,
    /// Terminal paste in most Linux terminal emulators.
    CtrlShiftV,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum DeepseekThinking {
    /// Do not send a thinking parameter.
    Auto,
    /// Ask DeepSeek to use thinking mode.
    Enabled,
    /// Ask DeepSeek to use non-thinking mode for faster cleanup.
    Disabled,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let input = read_input(&args)?;

    if input.trim().is_empty() {
        return Ok(());
    }

    let policy = cleanup_policy(&args);
    let outcome = cleanup_engine::clean_with_policy(
        &policy,
        input.trim(),
        if args.segment {
            CleanupKind::Segment
        } else {
            CleanupKind::Full
        },
        if args.strict {
            FailurePolicy::Strict
        } else {
            FailurePolicy::RawFallback
        },
    )?;
    if args.segment && !outcome.cleaned {
        return Ok(());
    }
    if !args.segment {
        if let Some(error) = &outcome.error {
            warn(
                &args,
                format_args!("LLM cleanup failed; using original text: {error}"),
            );
        }
    }
    let final_text = outcome.text.trim().to_owned();

    let stdout_only = args.stdout_only;
    if args.print || stdout_only {
        println!("{final_text}");
    }

    let paste_method = effective_paste_method(&args);
    let needs_clipboard_for_paste = paste_method == PasteMethod::Hotkey;

    if !stdout_only && (!args.no_copy || needs_clipboard_for_paste) {
        if let Err(err) = copy_to_clipboard(&final_text) {
            warn(&args, format_args!("clipboard copy failed: {err:#}"));
        }
    }

    if !stdout_only && paste_method != PasteMethod::None {
        if let Err(err) = paste_or_type_text(&args, paste_method, &final_text) {
            warn(
                &args,
                format_args!("inserting text into active window failed: {err:#}"),
            );
        }
    }

    Ok(())
}

fn read_input(args: &Args) -> Result<String> {
    if !args.text.is_empty() {
        return Ok(args.text.join(" "));
    }

    let mut stdin = io::stdin();
    if stdin.is_terminal() {
        return Err(anyhow!(
            "no input provided; pass text as arguments or pipe a transcript into this command"
        ));
    }

    let mut input = String::new();
    stdin
        .read_to_string(&mut input)
        .context("failed to read stdin")?;
    Ok(input)
}

fn cleanup_policy(args: &Args) -> CleanupPolicy {
    let explicit_key = args.api_key.clone().filter(|key| !key.is_empty());
    let provider_key = |name| {
        explicit_key
            .clone()
            .or_else(|| env::var(name).ok().filter(|key| !key.is_empty()))
    };
    CleanupPolicy {
        provider: if args.no_llm {
            "none".into()
        } else {
            args.provider.as_str().into()
        },
        endpoint: args.endpoint.clone().unwrap_or_default(),
        model: args.model.clone().unwrap_or_default(),
        credentials: CleanupCredentials {
            deepseek: provider_key("DEEPSEEK_API_KEY"),
            openai: provider_key("OPENAI_API_KEY"),
            ollama: provider_key("OLLAMA_API_KEY"),
            custom: explicit_key,
        },
        temperature: args.temperature,
        timeout_secs: args.timeout_secs,
        thinking: args.deepseek_thinking.as_str().into(),
        instructions: args
            .instructions
            .clone()
            .unwrap_or_else(|| pickscribe::config::DEFAULT_INSTRUCTIONS.into()),
        local_only: args.local_only,
    }
}

impl Provider {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Deepseek => "deepseek",
            Self::Ollama => "ollama",
            Self::Openai => "openai",
            Self::None => "none",
        }
    }
}

impl DeepseekThinking {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

fn copy_to_clipboard(text: &str) -> Result<()> {
    if command_exists("wl-copy") {
        // wl-copy may keep a helper process alive to serve the Wayland clipboard.
        // Do not wait forever; just feed it the text and let it manage the clipboard.
        run_with_stdin_no_wait("wl-copy", &[], text)
    } else if command_exists("xclip") {
        run_with_stdin("xclip", &["-selection", "clipboard"], text)
    } else if command_exists("xsel") {
        run_with_stdin("xsel", &["--clipboard", "--input"], text)
    } else {
        Err(anyhow!(
            "no clipboard helper found; install wl-clipboard, xclip, or xsel"
        ))
    }
}

fn effective_paste_method(args: &Args) -> PasteMethod {
    if args.no_paste {
        return PasteMethod::None;
    }

    match args.paste_method {
        PasteMethod::Auto => {
            if command_exists("ydotool") || command_exists("xdotool") {
                PasteMethod::Hotkey
            } else {
                PasteMethod::Type
            }
        }
        method => method,
    }
}

fn paste_or_type_text(args: &Args, method: PasteMethod, text: &str) -> Result<()> {
    if args.paste_delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(args.paste_delay_ms));
    }

    match method {
        PasteMethod::Hotkey => paste_with_hotkey(args),
        PasteMethod::Type => type_text(args, text),
        PasteMethod::Auto | PasteMethod::None => Ok(()),
    }
}

fn paste_with_hotkey(args: &Args) -> Result<()> {
    match choose_type_backend(args.type_backend) {
        Some(TypeBackend::Ydotool) => {
            // Release common modifiers first. This avoids shortcut keys like
            // Ctrl+Alt+Space or Ctrl+Shift+C affecting the paste action.
            // Linux input keycodes: Ctrl=29, Shift=42, V=47, Alt=56,
            // RightCtrl=97, RightShift=54, RightAlt=100, Meta=125/126.
            let chord: &[&str] = match args.paste_chord {
                PasteChord::CtrlV => &[
                    "key", "29:0", "97:0", "42:0", "54:0", "56:0", "100:0", "125:0", "126:0",
                    "29:1", "47:1", "47:0", "29:0",
                ],
                PasteChord::CtrlShiftV => &[
                    "key", "29:0", "97:0", "42:0", "54:0", "56:0", "100:0", "125:0", "126:0",
                    "29:1", "42:1", "47:1", "47:0", "42:0", "29:0",
                ],
            };
            run_status("ydotool", chord)
        }
        Some(TypeBackend::Xdotool) => {
            let chord = match args.paste_chord {
                PasteChord::CtrlV => "ctrl+v",
                PasteChord::CtrlShiftV => "ctrl+shift+v",
            };
            run_status("xdotool", &["key", "--clearmodifiers", chord])
        }
        Some(TypeBackend::Wtype) => Err(anyhow!(
            "wtype cannot send paste shortcuts; use --paste-method type or install ydotool"
        )),
        Some(TypeBackend::Auto | TypeBackend::None) | None => Err(anyhow!(
            "no paste hotkey backend found; install ydotool for Wayland or xdotool for X11"
        )),
    }
}

fn type_text(args: &Args, text: &str) -> Result<()> {
    match choose_type_backend(args.type_backend) {
        Some(TypeBackend::Ydotool) => run_with_stdin("ydotool", &["type", "--file", "-"], text),
        Some(TypeBackend::Xdotool) => run_with_stdin(
            "xdotool",
            &["type", "--clearmodifiers", "--file", "-"],
            text,
        ),
        Some(TypeBackend::Wtype) => run_status("wtype", &[text]),
        Some(TypeBackend::Auto | TypeBackend::None) | None => Err(anyhow!(
            "no typing backend found; install ydotool for Wayland, wtype if your compositor supports it, or xdotool for X11"
        )),
    }
}

fn choose_type_backend(requested: TypeBackend) -> Option<TypeBackend> {
    if requested != TypeBackend::Auto {
        return (requested != TypeBackend::None).then_some(requested);
    }

    let wayland = env::var("XDG_SESSION_TYPE")
        .map(|session| session.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false);

    let candidates: &[TypeBackend] = if wayland {
        &[
            TypeBackend::Ydotool,
            TypeBackend::Wtype,
            TypeBackend::Xdotool,
        ]
    } else {
        &[
            TypeBackend::Xdotool,
            TypeBackend::Ydotool,
            TypeBackend::Wtype,
        ]
    };

    candidates
        .iter()
        .copied()
        .find(|backend| command_exists(backend.command_name()))
}

impl TypeBackend {
    fn command_name(self) -> &'static str {
        match self {
            TypeBackend::Ydotool => "ydotool",
            TypeBackend::Wtype => "wtype",
            TypeBackend::Xdotool => "xdotool",
            TypeBackend::Auto | TypeBackend::None => "",
        }
    }
}

fn run_with_stdin(program: &str, args: &[&str], input: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {program}"))?;

    child
        .stdin
        .take()
        .context("failed to open child stdin")?
        .write_all(input.as_bytes())
        .with_context(|| format!("failed to write to {program} stdin"))?;

    let output = child
        .wait_with_output()
        .with_context(|| format!("failed to wait for {program}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "{program} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn run_with_stdin_no_wait(program: &str, args: &[&str], input: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start {program}"))?;

    child
        .stdin
        .take()
        .context("failed to open child stdin")?
        .write_all(input.as_bytes())
        .with_context(|| format!("failed to write to {program} stdin"))?;

    Ok(())
}

fn run_status(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to start {program}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "{program} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn command_exists(program: &str) -> bool {
    if program.is_empty() || program.contains('/') {
        return false;
    }

    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

fn warn(args: &Args, message: std::fmt::Arguments<'_>) {
    if !args.quiet {
        eprintln!("warning: {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(values: &[&str]) -> Args {
        Args::parse_from(values)
    }

    #[test]
    fn local_only_blocks_remote_cleanup_endpoint() {
        let args = parse_args(&[
            "pickscribe-cleanup",
            "--local-only",
            "--provider",
            "openai",
            "--api-key",
            "test-key",
            "hello",
        ]);

        let err = cleanup_engine::clean_with_policy(
            &cleanup_policy(&args),
            "hello",
            CleanupKind::Full,
            FailurePolicy::Strict,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("local-only mode blocks remote endpoint"));
    }

    #[test]
    fn local_only_blocks_ollama_cloud_models() {
        let args = parse_args(&[
            "pickscribe-cleanup",
            "--local-only",
            "--provider",
            "ollama",
            "--model",
            "deepseek-r1:cloud",
            "hello",
        ]);

        let err = cleanup_engine::clean_with_policy(
            &cleanup_policy(&args),
            "hello",
            CleanupKind::Full,
            FailurePolicy::Strict,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains(":cloud"));
    }

    #[test]
    fn local_only_auto_provider_prefers_ollama_even_with_remote_key() {
        let args = parse_args(&[
            "pickscribe-cleanup",
            "--local-only",
            "--provider",
            "auto",
            "--api-key",
            "remote-key",
            "hello",
        ]);

        let policy = cleanup_policy(&args);
        let resolved = cleanup_engine::resolve_policy(&policy).unwrap();

        assert_eq!(resolved.provider, "ollama");
    }
}
