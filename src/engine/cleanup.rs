use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AppConfig;

#[derive(Debug, Clone, Serialize)]
pub struct CleanupOutcome {
    pub text: String,
    pub provider: String,
    pub model: String,
    /// False when the LLM failed and we fell back to the raw transcript.
    pub cleaned: bool,
    pub error: Option<String>,
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
struct LlmTarget {
    provider: String,
    endpoint: String,
    model: String,
    api_key: Option<String>,
}

/// True when the endpoint points at this machine (loopback only).
pub fn is_local_endpoint(endpoint: &str) -> bool {
    let rest = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);
    let host_port = rest.split('/').next().unwrap_or("");
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        // Bracketed IPv6 literal, e.g. [::1]:11434
        stripped.split(']').next().unwrap_or("")
    } else {
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0")
}

fn resolve_target(cfg: &AppConfig) -> Result<LlmTarget> {
    let provider = cfg.effective_provider();
    let (endpoint, model) = match provider.as_str() {
        "deepseek" => (
            "https://api.deepseek.com/v1/chat/completions".to_string(),
            "deepseek-v4-flash".to_string(),
        ),
        "openai" => (
            "https://api.openai.com/v1/chat/completions".to_string(),
            "gpt-4o-mini".to_string(),
        ),
        "ollama" => {
            let host = std::env::var("OLLAMA_HOST")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "http://127.0.0.1:11434".into());
            (
                format!("{}/v1/chat/completions", host.trim_end_matches('/')),
                std::env::var("OLLAMA_MODEL")
                    .ok()
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| "qwen2.5:14b".into()),
            )
        }
        "custom" => {
            if cfg.cleanup.endpoint.is_empty() {
                bail!("custom provider needs an endpoint (full /chat/completions URL)");
            }
            // Model may stay empty while the user is still picking one;
            // try_clean validates it before sending a request.
            (cfg.cleanup.endpoint.clone(), cfg.cleanup.model.clone())
        }
        "none" => bail!("cleanup disabled"),
        other => bail!("unknown cleanup provider: {other}"),
    };

    let endpoint = if cfg.cleanup.endpoint.is_empty() {
        endpoint
    } else {
        cfg.cleanup.endpoint.clone()
    };
    let model = if cfg.cleanup.model.is_empty() {
        model
    } else {
        cfg.cleanup.model.clone()
    };
    if cfg.general.local_only && !is_local_endpoint(&endpoint) {
        bail!("local-only mode blocks remote endpoint {endpoint} — use Ollama or disable cleanup");
    }
    // Ollama ":cloud" models answer from ollama.com even though the endpoint
    // is loopback — they are not local.
    if cfg.general.local_only && model.ends_with(":cloud") {
        bail!("local-only mode blocks {model} — Ollama ':cloud' models run on ollama.com; pull a local model instead");
    }
    let api_key = cfg.resolve_api_key(&provider);
    if api_key.is_none() && matches!(provider.as_str(), "deepseek" | "openai") {
        bail!("no API key configured for provider {provider}");
    }
    Ok(LlmTarget {
        provider,
        endpoint,
        model,
        api_key,
    })
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
    let target = resolve_target(cfg)?;
    let url = models_url(&target.endpoint);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("building HTTP client")?;
    let mut builder = client.get(&url);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let response = builder.send().with_context(|| format!("GET {url} failed"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_endpoints_are_detected() {
        assert!(is_local_endpoint("http://127.0.0.1:11434/v1/chat/completions"));
        assert!(is_local_endpoint("http://localhost:8080/v1/chat/completions"));
        assert!(is_local_endpoint("http://[::1]:11434/v1/chat/completions"));
        assert!(!is_local_endpoint("https://api.deepseek.com/v1/chat/completions"));
        assert!(!is_local_endpoint("https://openrouter.ai/api/v1/chat/completions"));
        assert!(!is_local_endpoint("http://192.168.1.10:11434/v1/chat/completions"));
    }

    #[test]
    fn local_only_blocks_ollama_cloud_models() {
        let mut cfg = AppConfig::default();
        cfg.general.local_only = true;
        cfg.cleanup.provider = "ollama".into();
        cfg.cleanup.model = "deepseek-v4-flash:cloud".into();
        let err = resolve_target(&cfg).unwrap_err().to_string();
        assert!(err.contains(":cloud"), "unexpected error: {err}");

        cfg.cleanup.model = "qwen3.5:8b".into();
        assert!(resolve_target(&cfg).is_ok());
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
}

/// Clean `transcript` with the configured LLM. Never fails hard: on any error
/// the raw transcript is returned with `cleaned: false` and the error message.
pub fn clean(cfg: &AppConfig, transcript: &str) -> CleanupOutcome {
    if cfg.cleanup.provider == "none" {
        return CleanupOutcome {
            text: transcript.to_string(),
            provider: "none".into(),
            model: String::new(),
            cleaned: false,
            error: None,
        };
    }
    match try_clean(cfg, transcript) {
        Ok(outcome) => outcome,
        Err(err) => CleanupOutcome {
            text: transcript.to_string(),
            provider: cfg.effective_provider(),
            model: cfg.cleanup.model.clone(),
            cleaned: false,
            error: Some(format!("{err:#}")),
        },
    }
}

fn try_clean(cfg: &AppConfig, transcript: &str) -> Result<CleanupOutcome> {
    let target = resolve_target(cfg)?;
    if target.model.is_empty() {
        bail!("no model set for the custom provider — fetch or type one in Settings");
    }
    let instructions = cfg.effective_instructions();
    let user_content = format!("{instructions}\n\nText:\n{transcript}");

    let thinking = if target.provider == "deepseek" {
        match cfg.cleanup.thinking.as_str() {
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
                content: "You clean up dictated text for immediate pasting.",
            },
            ChatMessage {
                role: "user",
                content: &user_content,
            },
        ],
        temperature: cfg.cleanup.temperature,
        stream: false,
        thinking,
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(cfg.cleanup.timeout_secs.max(5)))
        .build()
        .context("building HTTP client")?;
    let mut builder = client.post(&target.endpoint).json(&request);
    if let Some(key) = &target.api_key {
        builder = builder.bearer_auth(key);
    }
    let response = builder.send().context("LLM request failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        bail!("LLM returned {status}: {}", body.chars().take(300).collect::<String>());
    }
    let parsed: ChatResponse = response.json().context("parsing LLM response")?;
    let text = parsed
        .choices
        .first()
        .map(|c| c.message.content.trim().to_string())
        .filter(|t| !t.is_empty())
        .context("LLM response contained no text")?;

    Ok(CleanupOutcome {
        text,
        provider: target.provider,
        model: target.model,
        cleaned: true,
        error: None,
    })
}
