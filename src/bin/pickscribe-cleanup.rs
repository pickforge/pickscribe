use anyhow::{Context, Result, anyhow};
use clap::{Parser, ValueEnum};
use pickscribe::engine::cleanup::{is_local_endpoint, segment_cleanup_is_safe};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::{self, IsTerminal, Read, Write},
    process::{Command, Stdio},
    time::Duration,
};

const DEFAULT_INSTRUCTIONS: &str = "Rewrite this dictated text so it is clean, natural, and ready to paste.\n\
Keep the original language. If the text is Portuguese, use natural Brazilian Portuguese.\n\
Fix punctuation, grammar, casing, and obvious speech-to-text mistakes.\n\
Do not add explanations.\n\
Return only the final text.";

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

#[derive(Debug)]
struct LlmConfig {
    provider: Provider,
    endpoint: String,
    model: String,
    api_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Thinking>,
}

#[derive(Debug, Serialize)]
struct Thinking {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    content: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let input = read_input(&args)?;

    if input.trim().is_empty() {
        return Ok(());
    }

    let final_text = if args.segment && (args.no_llm || args.provider == Provider::None) {
        return Ok(());
    } else if args.no_llm || args.provider == Provider::None {
        input.trim().to_owned()
    } else {
        match clean_with_llm(&args, input.trim()) {
            Ok(cleaned)
                if !cleaned.trim().is_empty()
                    && (!args.segment
                        || (cleaned.trim() != input.trim()
                            && segment_cleanup_is_safe(input.trim(), cleaned.trim()))) =>
            {
                cleaned.trim().to_owned()
            }
            Ok(_) if args.segment => return Ok(()),
            Ok(_) => input.trim().to_owned(),
            Err(err) if args.strict => return Err(err),
            Err(_) if args.segment => return Ok(()),
            Err(err) => {
                warn(
                    &args,
                    format_args!("LLM cleanup failed; using original text: {err:#}"),
                );
                input.trim().to_owned()
            }
        }
    };

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

fn clean_with_llm(args: &Args, text: &str) -> Result<String> {
    let config = resolve_llm_config(args)?;

    if matches!(config.provider, Provider::Deepseek | Provider::Openai) && config.api_key.is_none()
    {
        return Err(anyhow!(
            "missing API key for {:?}; set DEEPSEEK_API_KEY, OPENAI_API_KEY, or PICKSCRIBE_API_KEY",
            config.provider
        ));
    }

    let instructions = args.instructions.as_deref().unwrap_or(DEFAULT_INSTRUCTIONS);
    let (system_prompt, user_prompt) = if args.segment {
        (
            "You clean one short dictated transcript fragment for a live preview.".to_owned(),
            format!(
                "Cleanup instructions, spelling notes, and vocabulary:\n{instructions}\n\n\
Fragment:\n{text}\n\n\
Return only a conservative cleanup of this fragment. Do not add examples, \
complete unfinished thoughts, expand lists, or use words not supported by the fragment."
            ),
        )
    } else {
        (
            "You clean up dictated text for immediate pasting.".to_owned(),
            format!("{instructions}\n\nText:\n{text}"),
        )
    };

    let payload = ChatRequest {
        model: config.model,
        messages: vec![
            ChatMessage {
                role: "system".to_owned(),
                content: system_prompt,
            },
            ChatMessage {
                role: "user".to_owned(),
                content: user_prompt,
            },
        ],
        temperature: args.temperature,
        stream: false,
        thinking: deepseek_thinking_payload(args, config.provider),
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(args.timeout_secs))
        .build()
        .context("failed to build HTTP client")?;

    let mut request = client
        .post(&config.endpoint)
        .header("Content-Type", "application/json")
        .json(&payload);

    if let Some(api_key) = config.api_key.as_deref().filter(|key| !key.is_empty()) {
        request = request.bearer_auth(api_key);
    }

    let response = request
        .send()
        .with_context(|| format!("request to {} failed", config.endpoint))?;

    let status = response.status();
    let body = response.text().context("failed to read LLM response")?;

    if !status.is_success() {
        return Err(anyhow!("LLM returned HTTP {status}: {body}"));
    }

    let parsed: ChatResponse = serde_json::from_str(&body)
        .with_context(|| format!("failed to parse LLM response JSON: {body}"))?;

    parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .ok_or_else(|| anyhow!("LLM response had no choices"))
}

fn deepseek_thinking_payload(args: &Args, provider: Provider) -> Option<Thinking> {
    if provider != Provider::Deepseek {
        return None;
    }

    match args.deepseek_thinking {
        DeepseekThinking::Auto => None,
        DeepseekThinking::Enabled => Some(Thinking {
            kind: "enabled".to_owned(),
        }),
        DeepseekThinking::Disabled => Some(Thinking {
            kind: "disabled".to_owned(),
        }),
    }
}

fn resolve_llm_config(args: &Args) -> Result<LlmConfig> {
    let provider = match args.provider {
        Provider::Auto if args.local_only => Provider::Ollama,
        Provider::Auto => {
            if env::var_os("DEEPSEEK_API_KEY").is_some() || args.api_key.is_some() {
                Provider::Deepseek
            } else if env::var_os("OPENAI_API_KEY").is_some() {
                Provider::Openai
            } else {
                Provider::Ollama
            }
        }
        provider => provider,
    };

    let endpoint = match (&args.endpoint, provider) {
        (Some(endpoint), _) => endpoint.clone(),
        (None, Provider::Deepseek) => "https://api.deepseek.com/v1/chat/completions".to_owned(),
        (None, Provider::Openai) => "https://api.openai.com/v1/chat/completions".to_owned(),
        (None, Provider::Ollama) => ollama_endpoint(),
        (None, Provider::None | Provider::Auto) => return Err(anyhow!("invalid LLM provider")),
    };

    let model = args.model.clone().unwrap_or_else(|| match provider {
        Provider::Deepseek => "deepseek-v4-flash".to_owned(),
        Provider::Openai => "gpt-4o-mini".to_owned(),
        Provider::Ollama => env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5:14b".to_owned()),
        Provider::None | Provider::Auto => unreachable!(),
    });

    let api_key = args.api_key.clone().or_else(|| match provider {
        Provider::Deepseek => env::var("DEEPSEEK_API_KEY").ok(),
        Provider::Openai => env::var("OPENAI_API_KEY").ok(),
        Provider::Ollama => env::var("OLLAMA_API_KEY").ok(),
        Provider::None | Provider::Auto => None,
    });

    if args.local_only && !is_local_endpoint(&endpoint) {
        return Err(anyhow!(
            "local-only mode blocks remote endpoint {endpoint}; use Ollama or disable cleanup"
        ));
    }
    if args.local_only && model.ends_with(":cloud") {
        return Err(anyhow!(
            "local-only mode blocks {model}; Ollama ':cloud' models run outside this machine"
        ));
    }

    Ok(LlmConfig {
        provider,
        endpoint,
        model,
        api_key,
    })
}

fn ollama_endpoint() -> String {
    let host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_owned());
    let host = if host.starts_with("http://") || host.starts_with("https://") {
        host
    } else {
        format!("http://{host}")
    };
    format!("{}/v1/chat/completions", host.trim_end_matches('/'))
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

        let err = resolve_llm_config(&args).unwrap_err().to_string();

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

        let err = resolve_llm_config(&args).unwrap_err().to_string();

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

        let config = resolve_llm_config(&args).unwrap();

        assert_eq!(config.provider, Provider::Ollama);
        assert!(is_local_endpoint(&config.endpoint));
    }
}
