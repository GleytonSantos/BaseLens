//! AI assistant: local (Ollama, any OpenAI-compatible server), cloud
//! (Anthropic, OpenAI, Google) and the natively installed Claude Code CLI.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

pub const DEFAULT_TEMPERATURE: f32 = 0.7; // MECHANICS: config.ts defaultTemperature
pub const OLLAMA_URL: &str = "http://localhost:11434";
pub const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1";
pub const OPENAI_URL: &str = "https://api.openai.com/v1";
pub const GOOGLE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub const ANTHROPIC_MODELS: [&str; 4] = ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5", "claude-fable-5-1"];
pub const OPENAI_MODELS: [&str; 3] = ["gpt-5", "gpt-5-mini", "gpt-4.1"];
pub const GOOGLE_MODELS: [&str; 3] = ["gemini-3.5-flash", "gemini-3.5-flash-lite", "gemini-3.8-flash"];

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum Provider {
    #[default]
    Ollama,
    OpenAiCompat,
    ClaudeCode,
    Anthropic,
    OpenAi,
    Google,
}

impl Provider {
    pub const ALL: [Provider; 6] = [
        Provider::Ollama,
        Provider::OpenAiCompat,
        Provider::ClaudeCode,
        Provider::Anthropic,
        Provider::OpenAi,
        Provider::Google,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Provider::Ollama => "Ollama (local)",
            Provider::OpenAiCompat => "OpenAI-compatible (local / custom)",
            Provider::ClaudeCode => "Claude Code (installed CLI)",
            Provider::Anthropic => "Anthropic API",
            Provider::OpenAi => "OpenAI API",
            Provider::Google => "Google Gemini API",
        }
    }
    pub fn is_local(self) -> bool {
        matches!(self, Provider::Ollama | Provider::OpenAiCompat)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct AiSettings {
    pub provider: Provider,
    pub temperature: f32,
    pub ollama_url: String,
    pub ollama_model: String,
    pub compat_url: String,
    pub compat_model: String,
    pub compat_key_enc: String,
    pub anthropic_key_enc: String,
    pub anthropic_model: String,
    pub openai_key_enc: String,
    pub openai_model: String,
    pub google_key_enc: String,
    pub google_model: String,
    pub claude_path: String,
    pub claude_model: String,
    pub allow_read_only_auto: bool,
    pub custom_instructions: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            provider: Provider::Ollama,
            temperature: DEFAULT_TEMPERATURE,
            ollama_url: OLLAMA_URL.into(),
            ollama_model: String::new(),
            compat_url: "http://localhost:1234/v1".into(),
            compat_model: String::new(),
            compat_key_enc: String::new(),
            anthropic_key_enc: String::new(),
            anthropic_model: ANTHROPIC_MODELS[0].into(),
            openai_key_enc: String::new(),
            openai_model: OPENAI_MODELS[0].into(),
            google_key_enc: String::new(),
            google_model: GOOGLE_MODELS[0].into(),
            claude_path: String::new(),
            claude_model: String::new(),
            allow_read_only_auto: false,
            custom_instructions: String::new(),
        }
    }
}

impl AiSettings {
    pub fn model_label(&self) -> String {
        match self.provider {
            Provider::Ollama => self.ollama_model.clone(),
            Provider::OpenAiCompat => self.compat_model.clone(),
            Provider::ClaudeCode => {
                if self.claude_model.is_empty() { "default model".into() } else { self.claude_model.clone() }
            }
            Provider::Anthropic => self.anthropic_model.clone(),
            Provider::OpenAi => self.openai_model.clone(),
            Provider::Google => self.google_model.clone(),
        }
    }
}

/// Everything a request needs, with keys already decrypted.
#[derive(Clone)]
pub struct Call {
    pub provider: Provider,
    pub url: String,
    pub model: String,
    pub key: String,
    pub temperature: f32,
    pub claude_path: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    User,
    Assistant,
    Note,
}

#[derive(Clone, Debug)]
pub struct ChatMsg {
    pub role: Role,
    pub text: String,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
}

fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => {
            let body = r.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v.pointer("/error/message")
                        .or_else(|| v.get("error"))
                        .map(|m| m.as_str().map(|s| s.to_string()).unwrap_or_else(|| m.to_string()))
                })
                .unwrap_or(body);
            format!("HTTP {code}: {msg}")
        }
        other => format!("Could not reach the AI service: {other}"),
    }
}

pub fn ask(call: Call, system: String, history: Vec<ChatMsg>) -> Receiver<Result<String, String>> {
    let (tx, rx) = channel();
    if crate::store::robot_mode() {
        let _ = tx.send(Err("AI is switched off while the test robot runs.".into()));
        return rx;
    }
    std::thread::spawn(move || {
        let r = match call.provider {
            Provider::Ollama => {
                let url = format!("{}/v1/chat/completions", call.url.trim_end_matches('/'));
                openai_like(&url, &call, &system, &history)
            }
            Provider::OpenAiCompat => {
                let url = format!("{}/chat/completions", call.url.trim_end_matches('/'));
                openai_like(&url, &call, &system, &history)
            }
            Provider::OpenAi => openai_like(&format!("{OPENAI_URL}/chat/completions"), &call, &system, &history),
            Provider::Anthropic => anthropic(&call, &system, &history),
            Provider::Google => google(&call, &system, &history),
            Provider::ClaudeCode => claude_code(&call, &system, &history),
        };
        let _ = tx.send(r);
    });
    rx
}

fn convo(history: &[ChatMsg]) -> Vec<(&'static str, String)> {
    history
        .iter()
        .filter(|m| m.role != Role::Note)
        .map(|m| (if m.role == Role::User { "user" } else { "assistant" }, m.text.clone()))
        .collect()
}

fn openai_like(url: &str, call: &Call, system: &str, history: &[ChatMsg]) -> Result<String, String> {
    if call.model.is_empty() {
        return Err("Pick a model in Settings → AI first.".into());
    }
    let mut msgs = vec![json!({"role": "system", "content": system})];
    for (r, t) in convo(history) {
        msgs.push(json!({"role": r, "content": t}));
    }
    let body = json!({"model": call.model, "messages": msgs, "temperature": call.temperature, "stream": false});
    let mut req = agent().post(url).set("Content-Type", "application/json");
    if !call.key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", call.key));
    }
    let v: Value = req.send_json(body).map_err(http_err)?.into_json().map_err(|e| e.to_string())?;
    v.pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Unexpected answer: {v}"))
}

fn anthropic(call: &Call, system: &str, history: &[ChatMsg]) -> Result<String, String> {
    if call.key.is_empty() {
        return Err("Add your Anthropic API key in Settings → AI.".into());
    }
    let msgs: Vec<Value> = convo(history).into_iter().map(|(r, t)| json!({"role": r, "content": t})).collect();
    let body = json!({"model": call.model, "max_tokens": 4096, "system": system, "messages": msgs});
    let v: Value = agent()
        .post(&format!("{ANTHROPIC_URL}/messages"))
        .set("x-api-key", &call.key)
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .send_json(body)
        .map_err(http_err)?
        .into_json()
        .map_err(|e| e.to_string())?;
    let text: Vec<String> = v
        .get("content")
        .and_then(|c| c.as_array())
        .map(|a| a.iter().filter_map(|b| b.get("text").and_then(|t| t.as_str()).map(String::from)).collect())
        .unwrap_or_default();
    if text.is_empty() {
        Err(format!("Unexpected answer: {v}"))
    } else {
        Ok(text.join("\n"))
    }
}

fn google(call: &Call, system: &str, history: &[ChatMsg]) -> Result<String, String> {
    if call.key.is_empty() {
        return Err("Add your Google API key in Settings → AI.".into());
    }
    let contents: Vec<Value> = convo(history)
        .into_iter()
        .map(|(r, t)| json!({"role": if r == "user" { "user" } else { "model" }, "parts": [{"text": t}]}))
        .collect();
    let body = json!({
        "systemInstruction": {"parts": [{"text": system}]},
        "contents": contents,
        "generationConfig": {"temperature": call.temperature}
    });
    let v: Value = agent()
        .post(&format!("{GOOGLE_URL}/models/{}:generateContent", call.model))
        .set("x-goog-api-key", &call.key)
        .send_json(body)
        .map_err(http_err)?
        .into_json()
        .map_err(|e| e.to_string())?;
    let parts: Vec<String> = v
        .pointer("/candidates/0/content/parts")
        .and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str()).map(String::from)).collect())
        .unwrap_or_default();
    if parts.is_empty() {
        Err(format!("Unexpected answer: {v}"))
    } else {
        Ok(parts.join(""))
    }
}

fn claude_code(call: &Call, system: &str, history: &[ChatMsg]) -> Result<String, String> {
    let path = if call.claude_path.is_empty() {
        detect_claude().ok_or("Claude Code was not found. Install it or set its path in Settings → AI.")?
    } else {
        call.claude_path.clone()
    };
    let mut prompt = format!("{system}\n\n# Conversation\n");
    for (r, t) in convo(history) {
        prompt.push_str(&format!("\n## {}\n{}\n", if r == "user" { "User" } else { "Assistant" }, t));
    }
    prompt.push_str("\nReply to the last user message. Do not use tools; answer with text and SQL code blocks only.\n");
    let work = crate::store::data_dir().join("claude-workspace");
    let _ = std::fs::create_dir_all(&work);
    let mut cmd = std::process::Command::new(&path);
    cmd.arg("-p").arg("--output-format").arg("text");
    if !call.model.is_empty() {
        cmd.arg("--model").arg(&call.model);
    }
    let mut child = cmd
        .current_dir(&work)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start Claude Code at {path}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !text.is_empty() {
        Ok(text)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!("Claude Code failed: {}", if err.is_empty() { text } else { err }))
    }
}

/// Where a native Claude Code install usually lives (native installer first).
pub fn claude_candidates() -> Vec<String> {
    let mut v = Vec::new();
    if let Some(h) = dirs::home_dir() {
        v.push(h.join(".local/bin/claude").to_string_lossy().to_string());
        v.push(h.join(".claude/local/claude").to_string_lossy().to_string());
        v.push(h.join(".npm-global/bin/claude").to_string_lossy().to_string());
    }
    v.push("/usr/local/bin/claude".into());
    v.push("/usr/bin/claude".into());
    v.push("/opt/homebrew/bin/claude".into());
    v
}

pub fn detect_claude() -> Option<String> {
    for c in claude_candidates() {
        if std::path::Path::new(&c).exists() {
            return Some(c);
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for d in path.split(':') {
            let p = std::path::Path::new(d).join("claude");
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

pub fn claude_version(path: &str) -> Result<String, String> {
    let out = std::process::Command::new(path).arg("--version").output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn ollama_models(url: &str) -> Result<Vec<String>, String> {
    let v: Value = agent()
        .get(&format!("{}/api/tags", url.trim_end_matches('/')))
        .timeout(Duration::from_secs(4))
        .call()
        .map_err(http_err)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v.get("models")
        .and_then(|m| m.as_array())
        .map(|a| a.iter().filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from)).collect())
        .unwrap_or_default())
}

/// Pulls ```sql blocks (or any fenced block) out of an answer.
pub fn code_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = &rest[start + 3..];
        let body_start = after.find('\n').map(|i| i + 1).unwrap_or(0);
        let lang = after[..body_start].trim().to_lowercase();
        let body = &after[body_start..];
        let Some(end) = body.find("```") else { break };
        let code = body[..end].trim().to_string();
        if !code.is_empty() && (lang.is_empty() || lang.contains("sql")) {
            out.push(code);
        }
        rest = &body[end + 3..];
    }
    out
}
