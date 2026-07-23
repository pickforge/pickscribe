use anyhow::{Context, Result, anyhow};
use clap::{Parser, ValueEnum};
use pickscribe::engine::{
    cleanup::{
        self as cleanup_engine, CleanupCredentials, CleanupKind, CleanupPolicy, FailurePolicy,
    },
    paste::{
        self, DeliveryConfig, DeliveryMethod, PasteChord as EnginePasteChord,
        TypeBackend as EngineTypeBackend,
    },
};
use std::{
    env,
    io::{self, IsTerminal, Read},
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
    if !args.segment
        && let Some(error) = &outcome.error
    {
        warn(
            &args,
            format_args!("LLM cleanup failed; using original text: {error}"),
        );
    }
    let final_text = outcome.text.trim().to_owned();

    let stdout_only = args.stdout_only;
    if args.print || stdout_only {
        println!("{final_text}");
    }

    let delivery = paste::deliver(&delivery_config(&args), &final_text);
    if let Some(err) = delivery.clipboard_error {
        warn(&args, format_args!("clipboard copy failed: {err:#}"));
    }
    if let Some(err) = delivery.insertion_error {
        warn(
            &args,
            format_args!("inserting text into active window failed: {err:#}"),
        );
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

fn delivery_config(args: &Args) -> DeliveryConfig {
    let method = if args.stdout_only || args.no_paste {
        DeliveryMethod::None
    } else {
        match args.paste_method {
            PasteMethod::Auto => DeliveryMethod::Auto,
            PasteMethod::Hotkey => DeliveryMethod::Hotkey,
            PasteMethod::Type => DeliveryMethod::Type,
            PasteMethod::None => DeliveryMethod::None,
        }
    };

    DeliveryConfig {
        method,
        chord: match args.paste_chord {
            PasteChord::CtrlV => EnginePasteChord::CtrlV,
            PasteChord::CtrlShiftV => EnginePasteChord::CtrlShiftV,
        },
        delay_ms: args.paste_delay_ms,
        copy_to_clipboard: !args.stdout_only && !args.no_copy,
        type_backend: match args.type_backend {
            TypeBackend::Auto => EngineTypeBackend::Auto,
            TypeBackend::Ydotool => EngineTypeBackend::Ydotool,
            TypeBackend::Wtype => EngineTypeBackend::Wtype,
            TypeBackend::Xdotool => EngineTypeBackend::Xdotool,
            TypeBackend::None => EngineTypeBackend::None,
        },
    }
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
    fn stdout_only_disables_copy_and_insertion() {
        let args = parse_args(&["pickscribe-cleanup", "--stdout-only", "hello"]);

        let config = delivery_config(&args);

        assert_eq!(config.method, DeliveryMethod::None);
        assert!(!config.copy_to_clipboard);
    }

    #[test]
    fn no_copy_keeps_explicit_hotkey_for_shared_clipboard_policy() {
        let args = parse_args(&[
            "pickscribe-cleanup",
            "--no-copy",
            "--paste-method",
            "hotkey",
            "hello",
        ]);

        let config = delivery_config(&args);

        assert_eq!(config.method, DeliveryMethod::Hotkey);
        assert!(!config.copy_to_clipboard);
    }

    #[test]
    fn no_paste_still_allows_default_clipboard_copy() {
        let args = parse_args(&["pickscribe-cleanup", "--no-paste", "hello"]);

        let config = delivery_config(&args);

        assert_eq!(config.method, DeliveryMethod::None);
        assert!(config.copy_to_clipboard);
    }

    #[test]
    fn delivery_flags_map_chord_delay_and_custom_backend() {
        let args = parse_args(&[
            "pickscribe-cleanup",
            "--paste-method",
            "type",
            "--paste-chord",
            "ctrl-shift-v",
            "--paste-delay-ms",
            "275",
            "--type-backend",
            "xdotool",
            "hello",
        ]);

        let config = delivery_config(&args);

        assert_eq!(config.method, DeliveryMethod::Type);
        assert_eq!(config.chord, EnginePasteChord::CtrlShiftV);
        assert_eq!(config.delay_ms, 275);
        assert_eq!(config.type_backend, EngineTypeBackend::Xdotool);
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
