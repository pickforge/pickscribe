//! A small MCP server over stdio, so coding agents can fix the dictionary while the user talks
//! to them: when a dictated word looks wrong, the agent asks, and records the confirmed
//! correction. It only reads and edits `dictionary.txt`.

use std::io::{BufRead, Write};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::control;
use crate::dictionary::{Dictionary, Entry};

const INSTRUCTIONS: &str = "\
The user often dictates messages with PickScribe, so names and terms can arrive misheard. When a \
word in the user's message looks like a speech recognition error, such as an unknown project or \
product name that sounds like a known one, ask the user what they meant before relying on it. \
After the user confirms, call add_correction with the misheard and the meant spelling. Use \
add_term for a name that was spelled differently each time. Never add an entry the user did not \
confirm.";

pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&message),
            Err(_) => Some(error(&Value::Null, -32700, "parse error")),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Returns the reply to send, or None for notifications.
fn handle(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message["method"].as_str().unwrap_or_default();
    let params = &message["params"];
    let result = match method {
        "initialize" => json!({
            // Echo the client's version; this server uses nothing version specific.
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "pickscribe", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call(&params["name"], &params["arguments"]),
        _ => return Some(error(&id, -32601, "method not found")),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tools() -> Value {
    let text =
        json!({ "type": "string", "description": "The correct spelling, for example Pickforge." });
    json!([
        {
            "name": "add_correction",
            "description": "Teach PickScribe that speech recognition writes `heard` when the user says `meant`. Future dictations replace it. Call only after the user confirms the right word.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "heard": { "type": "string", "description": "The misheard text as it appeared, for example Bitforge." },
                    "meant": text,
                },
                "required": ["heard", "meant"],
            },
        },
        {
            "name": "add_term",
            "description": "Add a name or term that PickScribe should always spell exactly as given.",
            "inputSchema": {
                "type": "object",
                "properties": { "term": text },
                "required": ["term"],
            },
        },
        {
            "name": "remove_entry",
            "description": "Remove a term, or a correction when `heard` is given.",
            "inputSchema": {
                "type": "object",
                "properties": { "text": text, "heard": { "type": "string" } },
                "required": ["text"],
            },
        },
        {
            "name": "list_dictionary",
            "description": "List the PickScribe dictionary: terms, and corrections as heard -> meant.",
            "inputSchema": { "type": "object", "properties": {} },
        },
    ])
}

fn call(name: &Value, args: &Value) -> Value {
    let arg = |key: &str| args[key].as_str();
    let outcome = match name.as_str().unwrap_or_default() {
        "add_correction" => edit(|d| add(d, arg("meant"), arg("heard"), true)),
        "add_term" => edit(|d| add(d, arg("term"), None, false)),
        "remove_entry" => edit(|d| remove(d, arg("text"), arg("heard"))),
        "list_dictionary" => Dictionary::load().map(|d| listing(&d)),
        other => Err(anyhow::anyhow!("unknown tool {other}")),
    };
    match outcome {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
        Err(err) => {
            json!({ "content": [{ "type": "text", "text": format!("{err:#}") }], "isError": true })
        }
    }
}

fn edit(change: impl FnOnce(&mut Dictionary) -> Result<String>) -> Result<String> {
    let mut dictionary = Dictionary::load()?;
    let message = change(&mut dictionary)?;
    dictionary.save()?;
    control::notify_dictionary_changed();
    Ok(message)
}

fn add(
    dictionary: &mut Dictionary,
    text: Option<&str>,
    heard: Option<&str>,
    needs_heard: bool,
) -> Result<String> {
    let Some(text) = text else {
        bail!("missing the correct spelling");
    };
    if needs_heard && heard.is_none() {
        bail!("missing the misheard text");
    }
    let entry = Entry::new(text, heard)?;
    let label = label(&entry);
    Ok(if dictionary.add(entry) {
        format!("Added {label}.")
    } else {
        format!("{label} was already in the dictionary.")
    })
}

fn remove(dictionary: &mut Dictionary, text: Option<&str>, heard: Option<&str>) -> Result<String> {
    let Some(text) = text else {
        bail!("missing the text to remove");
    };
    let entry = Entry::new(text, heard)?;
    if !dictionary.remove(&entry) {
        bail!("{} is not in the dictionary", label(&entry));
    }
    Ok(format!("Removed {}.", label(&entry)))
}

fn label(entry: &Entry) -> String {
    match &entry.heard {
        Some(heard) => format!("\"{heard}\" -> \"{}\"", entry.text),
        None => format!("\"{}\"", entry.text),
    }
}

fn listing(dictionary: &Dictionary) -> String {
    if dictionary.entries.is_empty() {
        return "The dictionary is empty.".into();
    }
    dictionary
        .entries
        .iter()
        .map(label)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_echoes_the_protocol_version() {
        let reply = handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}})).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(reply["result"]["serverInfo"]["name"], "pickscribe");
    }

    #[test]
    fn notifications_get_no_reply_and_unknown_methods_fail() {
        assert!(handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        let reply = handle(&json!({"jsonrpc":"2.0","id":"x","method":"resources/list"})).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn tools_have_object_schemas() {
        let tools = tools();
        assert_eq!(tools.as_array().unwrap().len(), 4);
        assert!(
            tools
                .as_array()
                .unwrap()
                .iter()
                .all(|tool| tool["inputSchema"]["type"] == "object")
        );
    }

    #[test]
    fn corrections_need_both_spellings() {
        let mut dictionary = Dictionary::default();
        assert!(add(&mut dictionary, Some("Pickforge"), None, true).is_err());
        assert_eq!(
            add(&mut dictionary, Some("Pickforge"), Some("Bitforge"), true).unwrap(),
            "Added \"Bitforge\" -> \"Pickforge\"."
        );
        assert!(remove(&mut dictionary, Some("Pickforge"), None).is_err());
        assert!(remove(&mut dictionary, Some("Pickforge"), Some("Bitforge")).is_ok());
    }
}
