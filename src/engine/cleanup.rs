use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{AppConfig, DEFAULT_INSTRUCTIONS};

#[derive(Debug, Clone, Serialize)]
pub struct CleanupOutcome {
    pub text: String,
    pub provider: String,
    pub model: String,
    /// False when cleanup was disabled, failed, or was rejected as unsafe.
    pub cleaned: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CleanupCredentials {
    pub deepseek: Option<String>,
    pub openai: Option<String>,
    pub ollama: Option<String>,
    pub custom: Option<String>,
}

impl CleanupCredentials {
    fn for_provider(&self, provider: &str) -> Option<String> {
        match provider {
            "deepseek" => self.deepseek.clone(),
            "openai" => self.openai.clone(),
            "ollama" => self.ollama.clone(),
            "custom" => self.custom.clone(),
            _ => None,
        }
        .filter(|key| !key.is_empty())
    }
}

#[derive(Debug, Clone)]
pub struct CleanupPolicy {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub credentials: CleanupCredentials,
    pub temperature: f32,
    pub timeout_secs: u64,
    pub thinking: String,
    pub instructions: String,
    pub local_only: bool,
}

impl CleanupPolicy {
    pub fn from_app_config(cfg: &AppConfig) -> Self {
        Self {
            provider: cfg.cleanup.provider.clone(),
            endpoint: cfg.cleanup.endpoint.clone(),
            model: cfg.cleanup.model.clone(),
            credentials: CleanupCredentials {
                deepseek: cfg.resolve_api_key("deepseek"),
                openai: cfg.resolve_api_key("openai"),
                ollama: cfg.resolve_api_key("ollama"),
                custom: cfg.resolve_api_key("custom"),
            },
            temperature: cfg.cleanup.temperature,
            // Preserve the desktop path's existing lower bound. The CLI maps
            // its timeout flag directly when it constructs this policy.
            timeout_secs: cfg.cleanup.timeout_secs.max(5),
            thinking: cfg.cleanup.thinking.clone(),
            instructions: if cfg.cleanup.instructions.trim().is_empty() {
                DEFAULT_INSTRUCTIONS.to_string()
            } else {
                cfg.cleanup.instructions.clone()
            },
            local_only: cfg.general.local_only,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum CleanupKind {
    Full,
    Segment,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FailurePolicy {
    RawFallback,
    Strict,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Value>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
    content: String,
}

#[derive(Debug)]
pub struct ResolvedCleanup {
    pub provider: String,
    endpoint: String,
    pub model: String,
    api_key: Option<String>,
}

/// True when the endpoint points at this machine (loopback only).
fn is_local_endpoint(endpoint: &str) -> bool {
    let rest = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);
    let host_port = rest.split('/').next().unwrap_or("");
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
}

fn effective_provider(policy: &CleanupPolicy) -> String {
    if policy.provider != "auto" {
        return policy.provider.clone();
    }
    if policy.local_only {
        return "ollama".into();
    }
    if policy.credentials.deepseek.is_some() {
        "deepseek".into()
    } else if policy.credentials.openai.is_some() {
        "openai".into()
    } else {
        "ollama".into()
    }
}

pub fn resolve_policy(policy: &CleanupPolicy) -> Result<ResolvedCleanup> {
    let provider = effective_provider(policy);
    let (default_endpoint, default_model) = match provider.as_str() {
        "deepseek" => (
            "https://api.deepseek.com/v1/chat/completions".to_string(),
            "deepseek-v4-flash".to_string(),
        ),
        "openai" => (
            "https://api.openai.com/v1/chat/completions".to_string(),
            "gpt-4o-mini".to_string(),
        ),
        "ollama" => (ollama_endpoint(), default_ollama_model()),
        "custom" => {
            if policy.endpoint.is_empty() {
                bail!("custom provider needs an endpoint (full /chat/completions URL)");
            }
            (policy.endpoint.clone(), policy.model.clone())
        }
        "none" => bail!("cleanup disabled"),
        other => bail!("unknown cleanup provider: {other}"),
    };

    let endpoint = if policy.endpoint.is_empty() {
        default_endpoint
    } else {
        policy.endpoint.clone()
    };
    let model = if policy.model.is_empty() {
        default_model
    } else {
        policy.model.clone()
    };

    if policy.local_only && !is_local_endpoint(&endpoint) {
        bail!("local-only mode blocks remote endpoint {endpoint} — use Ollama or disable cleanup");
    }
    if policy.local_only && model.ends_with(":cloud") {
        bail!(
            "local-only mode blocks {model} — Ollama ':cloud' models run on ollama.com; pull a local model instead"
        );
    }
    let api_key = policy.credentials.for_provider(&provider);
    if api_key.is_none() && matches!(provider.as_str(), "deepseek" | "openai") {
        bail!("no API key configured for provider {provider}");
    }

    Ok(ResolvedCleanup {
        provider,
        endpoint,
        model,
        api_key,
    })
}

fn resolve(cfg: &AppConfig) -> Result<ResolvedCleanup> {
    resolve_policy(&CleanupPolicy::from_app_config(cfg))
}

fn ollama_endpoint() -> String {
    let host = std::env::var("OLLAMA_HOST")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:11434".into());
    let host = if host.starts_with("http://") || host.starts_with("https://") {
        host
    } else {
        format!("http://{host}")
    };
    format!("{}/v1/chat/completions", host.trim_end_matches('/'))
}

fn default_ollama_model() -> String {
    std::env::var("OLLAMA_MODEL")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "qwen2.5:14b".into())
}

/// Derive the provider's /models URL from its chat completions endpoint.
fn models_url(chat_endpoint: &str) -> String {
    let trimmed = chat_endpoint.trim_end_matches('/');
    match trimmed.strip_suffix("/chat/completions") {
        Some(base) => format!("{base}/models"),
        None => format!("{trimmed}/models"),
    }
}

/// Ask the configured provider which models it serves (OpenAI-compatible
/// `GET /models`; Ollama's native `{"models": [...]}` shape is accepted too).
pub fn list_models(cfg: &AppConfig) -> Result<Vec<String>> {
    let target = resolve(cfg)?;
    let url = models_url(&target.endpoint);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("building HTTP client")?;
    let mut builder = client.get(&url);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let response = builder
        .send()
        .with_context(|| format!("GET {url} failed"))?;
    let status = response.status();
    if !status.is_success() {
        bail!("{url} returned {status}");
    }
    let body: Value = response.json().context("parsing models response")?;
    let items = body
        .get("data")
        .or_else(|| body.get("models"))
        .and_then(Value::as_array)
        .context("no model list in response")?;
    let mut models: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("id")
                .or_else(|| item.get("name"))
                .or_else(|| item.get("model"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    models.sort();
    models.dedup();
    if models.is_empty() {
        bail!("provider returned an empty model list");
    }
    Ok(models)
}

/// Apply cleanup policy through one interface. In strict mode, provider,
/// privacy, request, and malformed-response failures are returned to the
/// adapter. Raw-fallback mode records the error and returns the transcript.
pub fn clean_with_policy(
    policy: &CleanupPolicy,
    transcript: &str,
    kind: CleanupKind,
    failure_policy: FailurePolicy,
) -> Result<CleanupOutcome> {
    let provider = effective_provider(policy);
    if provider == "none" {
        return Ok(raw_outcome(transcript, provider, String::new(), None));
    }
    if transcript.trim().is_empty() {
        return Ok(raw_outcome(
            transcript,
            provider,
            policy.model.clone(),
            None,
        ));
    }

    let target = match resolve_policy(policy) {
        Ok(target) => target,
        Err(err) => return handle_failure(policy, transcript, failure_policy, err),
    };
    if target.model.is_empty() {
        return handle_resolved_failure(
            transcript,
            failure_policy,
            &target,
            anyhow::anyhow!("no model set for the custom provider — fetch or type one in Settings"),
        );
    }
    let cleaned = match request_cleanup(policy, &target, transcript, kind) {
        Ok(cleaned) => cleaned,
        Err(err) => {
            return handle_resolved_failure(transcript, failure_policy, &target, err);
        }
    };

    let cleaned = cleaned.trim();
    match kind {
        CleanupKind::Full if cleaned.is_empty() => handle_resolved_failure(
            transcript,
            failure_policy,
            &target,
            anyhow::anyhow!("LLM response contained no text"),
        ),
        CleanupKind::Full => Ok(cleaned_outcome(&target, cleaned)),
        CleanupKind::Segment => Ok(interpret_segment(&target, transcript, cleaned)),
    }
}

/// Clean `transcript` with the configured LLM. Never fails hard: on any error
/// the raw transcript is returned with `cleaned: false` and the error message.
pub fn clean(cfg: &AppConfig, transcript: &str) -> CleanupOutcome {
    let policy = CleanupPolicy::from_app_config(cfg);
    clean_with_policy(
        &policy,
        transcript,
        CleanupKind::Full,
        FailurePolicy::RawFallback,
    )
    .expect("raw-fallback cleanup policy cannot return an error")
}

/// Clean a finalized incremental fragment conservatively. Segment cleanup must
/// not add examples, complete thoughts, or expand lists from user instructions.
pub fn clean_segment(cfg: &AppConfig, transcript: &str) -> CleanupOutcome {
    let policy = CleanupPolicy::from_app_config(cfg);
    clean_with_policy(
        &policy,
        transcript,
        CleanupKind::Segment,
        FailurePolicy::RawFallback,
    )
    .expect("raw-fallback cleanup policy cannot return an error")
}

fn handle_failure(
    policy: &CleanupPolicy,
    transcript: &str,
    failure_policy: FailurePolicy,
    error: anyhow::Error,
) -> Result<CleanupOutcome> {
    if failure_policy == FailurePolicy::Strict {
        return Err(error);
    }
    Ok(raw_outcome(
        transcript,
        effective_provider(policy),
        policy.model.clone(),
        Some(format!("{error:#}")),
    ))
}

fn handle_resolved_failure(
    transcript: &str,
    failure_policy: FailurePolicy,
    target: &ResolvedCleanup,
    error: anyhow::Error,
) -> Result<CleanupOutcome> {
    if failure_policy == FailurePolicy::Strict {
        return Err(error);
    }
    Ok(raw_outcome(
        transcript,
        target.provider.clone(),
        target.model.clone(),
        Some(format!("{error:#}")),
    ))
}

fn raw_outcome(
    transcript: &str,
    provider: String,
    model: String,
    error: Option<String>,
) -> CleanupOutcome {
    CleanupOutcome {
        text: transcript.to_string(),
        provider,
        model,
        cleaned: false,
        error,
    }
}

fn cleaned_outcome(target: &ResolvedCleanup, text: &str) -> CleanupOutcome {
    CleanupOutcome {
        text: text.to_string(),
        provider: target.provider.clone(),
        model: target.model.clone(),
        cleaned: true,
        error: None,
    }
}

fn interpret_segment(target: &ResolvedCleanup, transcript: &str, cleaned: &str) -> CleanupOutcome {
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == transcript.trim() {
        raw_outcome(
            transcript,
            target.provider.clone(),
            target.model.clone(),
            None,
        )
    } else if segment_cleanup_is_safe(transcript, cleaned) {
        cleaned_outcome(target, cleaned)
    } else {
        raw_outcome(
            transcript,
            target.provider.clone(),
            target.model.clone(),
            Some("segment cleanup output diverged from the source fragment".into()),
        )
    }
}

fn request_cleanup(
    policy: &CleanupPolicy,
    target: &ResolvedCleanup,
    transcript: &str,
    kind: CleanupKind,
) -> Result<String> {
    let (system_content, user_content) = prompts(policy, transcript, kind);
    let thinking = if target.provider == "deepseek" {
        match policy.thinking.as_str() {
            "enabled" => Some(serde_json::json!({"type": "enabled"})),
            "disabled" => Some(serde_json::json!({"type": "disabled"})),
            _ => None,
        }
    } else {
        None
    };
    let request = ChatRequest {
        model: &target.model,
        messages: vec![
            ChatMessage {
                role: "system",
                content: &system_content,
            },
            ChatMessage {
                role: "user",
                content: &user_content,
            },
        ],
        temperature: policy.temperature,
        stream: false,
        thinking,
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(policy.timeout_secs))
        .build()
        .context("building HTTP client")?;
    let mut builder = client.post(&target.endpoint).json(&request);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let response = builder.send().context("LLM request failed")?;
    let status = response.status();
    let body = response.text().context("reading LLM response")?;
    if !status.is_success() {
        bail!(
            "LLM returned {status}: {}",
            body.chars().take(300).collect::<String>()
        );
    }
    parse_response(&body)
}

fn prompts(policy: &CleanupPolicy, transcript: &str, kind: CleanupKind) -> (String, String) {
    match kind {
        CleanupKind::Full => (
            "You clean up dictated text for immediate pasting.".into(),
            format!("{}\n\nText:\n{transcript}", policy.instructions),
        ),
        CleanupKind::Segment => (
            "You clean one short dictated transcript fragment for a live preview.".into(),
            format!(
                "Cleanup instructions, spelling notes, and vocabulary:\n{}\n\n\
Fragment:\n{transcript}\n\n\
Return only a conservative cleanup of this fragment. Do not add examples, \
complete unfinished thoughts, expand lists, or use words not supported by the fragment.",
                policy.instructions
            ),
        ),
    }
}

fn parse_response(body: &str) -> Result<String> {
    let parsed: ChatResponse = serde_json::from_str(body).with_context(|| {
        format!(
            "failed to parse LLM response JSON: {}",
            body.chars().take(300).collect::<String>()
        )
    })?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .context("LLM response had no choices")
}

pub fn segment_cleanup_is_safe(raw: &str, cleaned: &str) -> bool {
    let raw_tokens = normalized_tokens(raw);
    !raw_tokens.is_empty() && raw_tokens == normalized_tokens(cleaned)
}

fn normalized_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(normalized_token)
        .collect()
}

fn normalized_token(word: &str) -> Option<String> {
    let lower: String = word.chars().flat_map(char::to_lowercase).collect();
    let trimmed = lower.trim_matches(is_boundary_punctuation).to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn is_boundary_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '.' | ','
            | ';'
            | ':'
            | '!'
            | '?'
            | '"'
            | '\''
            | '`'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '-'
            | '—'
            | '–'
            | '“'
            | '”'
            | '‘'
            | '’'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(provider: &str) -> CleanupPolicy {
        CleanupPolicy {
            provider: provider.into(),
            endpoint: String::new(),
            model: String::new(),
            credentials: CleanupCredentials::default(),
            temperature: 0.2,
            timeout_secs: 30,
            thinking: "disabled".into(),
            instructions: DEFAULT_INSTRUCTIONS.into(),
            local_only: false,
        }
    }

    fn target() -> ResolvedCleanup {
        ResolvedCleanup {
            provider: "ollama".into(),
            endpoint: "http://127.0.0.1:11434/v1/chat/completions".into(),
            model: "qwen2.5:14b".into(),
            api_key: None,
        }
    }

    #[test]
    fn local_endpoints_are_detected() {
        assert!(is_local_endpoint(
            "http://127.0.0.1:11434/v1/chat/completions"
        ));
        assert!(is_local_endpoint(
            "http://localhost:8080/v1/chat/completions"
        ));
        assert!(is_local_endpoint("http://[::1]:11434/v1/chat/completions"));
        assert!(!is_local_endpoint(
            "https://api.deepseek.com/v1/chat/completions"
        ));
        assert!(!is_local_endpoint(
            "https://openrouter.ai/api/v1/chat/completions"
        ));
        assert!(!is_local_endpoint(
            "http://192.168.1.10:11434/v1/chat/completions"
        ));
    }

    #[test]
    fn auto_provider_uses_available_keys_and_local_only_prefers_ollama() {
        let mut cleanup = policy("auto");
        assert_eq!(effective_provider(&cleanup), "ollama");

        cleanup.credentials.openai = Some("openai-key".into());
        assert_eq!(effective_provider(&cleanup), "openai");

        cleanup.credentials.deepseek = Some("deepseek-key".into());
        assert_eq!(effective_provider(&cleanup), "deepseek");

        cleanup.local_only = true;
        assert_eq!(effective_provider(&cleanup), "ollama");
    }

    #[test]
    fn local_only_blocks_remote_endpoints_and_cloud_models() {
        let mut cleanup = policy("openai");
        cleanup.local_only = true;
        cleanup.credentials.openai = Some("key".into());
        let err = resolve_policy(&cleanup).unwrap_err().to_string();
        assert!(err.contains("local-only mode blocks remote endpoint"));

        cleanup.provider = "ollama".into();
        cleanup.endpoint = "http://127.0.0.1:11434/v1/chat/completions".into();
        cleanup.model = "deepseek-v4-flash:cloud".into();
        let err = resolve_policy(&cleanup).unwrap_err().to_string();
        assert!(err.contains(":cloud"), "unexpected error: {err}");

        cleanup.model = "qwen3.5:8b".into();
        assert!(resolve_policy(&cleanup).is_ok());
    }

    #[test]
    fn custom_target_can_resolve_before_a_model_is_selected() {
        let mut cleanup = policy("custom");
        cleanup.endpoint = "http://127.0.0.1:1234/v1/chat/completions".into();

        let target = resolve_policy(&cleanup).unwrap();
        assert!(target.model.is_empty());

        let error = clean_with_policy(&cleanup, "hello", CleanupKind::Full, FailurePolicy::Strict)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no model set for the custom provider"));
    }

    #[test]
    fn models_url_derived_from_chat_endpoint() {
        assert_eq!(
            models_url("https://api.deepseek.com/v1/chat/completions"),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:11434/v1/chat/completions/"),
            "http://127.0.0.1:11434/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:1234/v1"),
            "http://127.0.0.1:1234/v1/models"
        );
    }

    #[test]
    fn clean_returns_raw_transcript_when_cleanup_is_disabled() {
        let cleanup = policy("none");
        let outcome = clean_with_policy(
            &cleanup,
            "raw transcript",
            CleanupKind::Full,
            FailurePolicy::Strict,
        )
        .unwrap();

        assert_eq!(outcome.text, "raw transcript");
        assert_eq!(outcome.provider, "none");
        assert!(outcome.model.is_empty());
        assert!(!outcome.cleaned);
        assert!(outcome.error.is_none());
    }

    #[test]
    fn target_failure_is_strict_or_raw_fallback_by_policy() {
        let mut cleanup = policy("custom");
        cleanup.local_only = true;
        cleanup.endpoint = "https://example.com/v1/chat/completions".into();
        cleanup.model = "remote-model".into();

        let fallback = clean_with_policy(
            &cleanup,
            "keep this",
            CleanupKind::Full,
            FailurePolicy::RawFallback,
        )
        .unwrap();
        assert_eq!(fallback.text, "keep this");
        assert_eq!(fallback.provider, "custom");
        assert_eq!(fallback.model, "remote-model");
        assert!(!fallback.cleaned);
        assert!(
            fallback
                .error
                .as_deref()
                .is_some_and(|error| error.contains("local-only mode blocks remote endpoint"))
        );

        let strict = clean_with_policy(
            &cleanup,
            "keep this",
            CleanupKind::Full,
            FailurePolicy::Strict,
        )
        .unwrap_err()
        .to_string();
        assert!(strict.contains("local-only mode blocks remote endpoint"));
    }

    #[test]
    fn prompt_policy_distinguishes_full_and_conservative_segment_cleanup() {
        let cleanup = policy("ollama");
        let (full_system, full_user) = prompts(&cleanup, "hello", CleanupKind::Full);
        assert!(full_system.contains("immediate pasting"));
        assert!(full_user.ends_with("Text:\nhello"));

        let (segment_system, segment_user) = prompts(&cleanup, "hello", CleanupKind::Segment);
        assert!(segment_system.contains("live preview"));
        assert!(segment_user.contains("Fragment:\nhello"));
        assert!(segment_user.contains("Do not add examples"));
    }

    #[test]
    fn response_interpretation_uses_first_choice_and_rejects_missing_choices() {
        assert_eq!(
            parse_response(r#"{"choices":[{"message":{"content":" cleaned "}}]}"#).unwrap(),
            " cleaned "
        );
        assert!(parse_response(r#"{"choices":[]}"#).is_err());

        let malformed = "x".repeat(1_000);
        let error = parse_response(&malformed).unwrap_err().to_string();
        assert!(
            error.len() < 400,
            "unexpected unbounded error: {}",
            error.len()
        );
    }

    #[test]
    fn segment_interpretation_rejects_unchanged_unsafe_and_no_text_output() {
        let target = target();
        let raw = "hello segment";

        let unchanged = interpret_segment(&target, raw, raw);
        assert!(!unchanged.cleaned);
        assert!(unchanged.error.is_none());

        let no_text = interpret_segment(&target, raw, "  ");
        assert!(!no_text.cleaned);
        assert!(no_text.error.is_none());

        let unsafe_output = interpret_segment(&target, raw, "hello segment PickScribe");
        assert!(!unsafe_output.cleaned);
        assert!(unsafe_output.error.is_some());

        let safe = interpret_segment(&target, raw, "Hello, segment.");
        assert!(safe.cleaned);
        assert_eq!(safe.text, "Hello, segment.");
    }

    #[test]
    fn segment_cleanup_safety_allows_conservative_edits() {
        assert!(segment_cleanup_is_safe(
            "okay so now maybe if i speak here",
            "Okay, so now maybe if I speak here."
        ));
        assert!(segment_cleanup_is_safe(
            "this is for pickforge and pickgauge",
            "This is for PickForge and PickGauge."
        ));
    }

    #[test]
    fn segment_cleanup_safety_rejects_instruction_example_leaks() {
        let raw = "to see of how this will work";
        let cleaned = "Pickforge Studio\nPickForge\nPickScribe\nPickGauge\nPickArena\nPickLab";
        assert!(!segment_cleanup_is_safe(raw, cleaned));
    }

    #[test]
    fn segment_cleanup_safety_rejects_boilerplate_expansions() {
        assert!(!segment_cleanup_is_safe(
            "just stop to talk",
            "Here's a cleaned and structured version of your dictated text: just stop to talk"
        ));
    }

    #[test]
    fn segment_cleanup_safety_rejects_short_fragment_growth() {
        assert!(!segment_cleanup_is_safe("okay", "Okay PickScribe"));
        assert!(!segment_cleanup_is_safe("okay", "Okay okay okay okay"));
        assert!(!segment_cleanup_is_safe(
            "to see of",
            "To see of PickScribe PickGauge PickLab"
        ));
    }

    #[test]
    fn segment_cleanup_safety_rejects_longer_boilerplate_suffixes() {
        assert!(!segment_cleanup_is_safe(
            "okay so now maybe if i speak here with text",
            "Okay, so now maybe if I speak here with text. Here's a cleaned and structured version."
        ));
    }

    #[test]
    fn segment_cleanup_safety_rejects_single_instruction_token_leak() {
        assert!(!segment_cleanup_is_safe(
            "okay so now maybe if",
            "Okay so now maybe PickGauge"
        ));
    }

    #[test]
    fn segment_cleanup_safety_rejects_duplicated_source_tokens() {
        assert!(!segment_cleanup_is_safe(
            "this segment should preserve every source token",
            "This segment segment should preserve every every source token"
        ));
    }

    #[test]
    fn segment_cleanup_safety_rejects_common_cleaned_text_prefixes() {
        assert!(!segment_cleanup_is_safe(
            "please send this email after review",
            "Cleaned text: please send this email after review"
        ));
    }

    #[test]
    fn segment_cleanup_safety_rejects_deleted_source_tokens() {
        assert!(!segment_cleanup_is_safe(
            "please do not send this email",
            "Please do send this email"
        ));
    }

    #[test]
    fn segment_cleanup_safety_preserves_word_boundaries() {
        assert!(!segment_cleanup_is_safe("now here", "nowhere"));
        assert!(!segment_cleanup_is_safe("therapist", "the rapist"));
    }

    #[test]
    fn segment_cleanup_safety_allows_dictated_boilerplate_phrases() {
        assert!(segment_cleanup_is_safe(
            "let me know if you have questions",
            "Let me know if you have questions."
        ));
    }

    #[test]
    fn segment_cleanup_safety_preserves_meaningful_symbols() {
        assert!(!segment_cleanup_is_safe("use C++ here", "Use C here."));
        assert!(!segment_cleanup_is_safe("use C# here", "Use C here."));
        assert!(!segment_cleanup_is_safe(
            "email foo@example.com today",
            "Email fooexamplecom today."
        ));
        assert!(!segment_cleanup_is_safe(
            "open https://pickforge.dev/pickscribe",
            "Open httpspickforgedevpickscribe."
        ));
    }
}
