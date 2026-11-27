use std::cell::Cell;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::event::{AgentRole, Event, TokenUsage};

/// Caps on a model-authored file manifest. A response beyond these is treated
/// as malformed: the agent cannot dump an unbounded tree into `project/`.
pub const MAX_FILES: usize = 512;
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

const SYSTEM_PROMPT: &str = "You are the executing agent of the Evengate software factory. \
You return complete, compilable projects. You never ask questions and never wrap the result in prose.";

/// One request to a model. The prompt is the only recorded input; its hash is
/// what `AgentInvoked` stores and what `RecordedClient` matches against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmRequest {
    pub role: AgentRole,
    pub prompt: String,
}

impl LlmRequest {
    pub fn prompt_hash(&self) -> String {
        format!("{:016x}", fnv1a64(self.prompt.as_bytes()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmResponse {
    pub text: String,
    pub tokens: TokenUsage,
}

/// The model boundary. Implementations are the only code allowed to produce
/// non-determinism here; every response is recorded as an event by the
/// executor, so replay never needs to call one.
pub trait LlmClient {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, String>;
}

/// The agent output protocol: one JSON object mapping relative paths to file
/// contents. Deterministic, diffable, and small enough to record verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileManifest {
    pub files: BTreeMap<String, String>,
}

impl FileManifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: FileManifest = serde_json::from_str(strip_fence(text))
            .map_err(|e| format!("agent response is not a file manifest: {e}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.files.is_empty() {
            return Err("agent manifest contains no files".into());
        }
        if self.files.len() > MAX_FILES {
            return Err(format!(
                "agent manifest has {} files (max {MAX_FILES})",
                self.files.len()
            ));
        }
        let mut total = 0usize;
        for (path, content) in &self.files {
            validate_path(path)?;
            let bytes = content.len();
            if bytes > MAX_FILE_BYTES {
                return Err(format!(
                    "manifest file `{path}` is {bytes} bytes (max {MAX_FILE_BYTES})"
                ));
            }
            total = total
                .checked_add(bytes)
                .ok_or("agent manifest size overflow")?;
        }
        if total > MAX_TOTAL_BYTES {
            return Err(format!(
                "agent manifest is {total} bytes (max {MAX_TOTAL_BYTES})"
            ));
        }
        Ok(())
    }
}

fn strip_fence(text: &str) -> &str {
    let text = text.trim();
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    let rest = rest.trim_start_matches(['\r', '\n']);
    let rest = rest.strip_suffix("```").unwrap_or(rest);
    rest.trim()
}

fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("manifest contains an empty path".into());
    }
    if path.len() > 512 {
        return Err(format!("manifest path `{path}` is longer than 512 bytes"));
    }
    if path.contains('\\') {
        return Err(format!(
            "manifest path `{path}` uses `\\`; the protocol requires `/`"
        ));
    }
    match Path::new(path).components().next() {
        Some(Component::RootDir) | Some(Component::Prefix(_)) => {
            return Err(format!("manifest path `{path}` is absolute"));
        }
        _ => {}
    }
    for segment in path.split('/') {
        match segment {
            "" => return Err(format!("manifest path `{path}` contains an empty segment")),
            "." => return Err(format!("manifest path `{path}` contains `.`")),
            ".." => return Err(format!("manifest path `{path}` escapes the project with `..`")),
            _ => {}
        }
    }
    Ok(())
}

/// Deterministic offline client: renders `templates/` into a manifest. This is
/// the seed kernel's agent and the default when no LLM endpoint is configured.
pub struct TemplateClient {
    templates: PathBuf,
    crate_name: String,
}

impl TemplateClient {
    pub fn new(templates: PathBuf, crate_name: String) -> Self {
        TemplateClient {
            templates,
            crate_name,
        }
    }
}

impl LlmClient for TemplateClient {
    fn complete(&self, _request: &LlmRequest) -> Result<LlmResponse, String> {
        let manifest = FileManifest {
            files: read_template_files(&self.templates, &self.crate_name)?,
        };
        manifest.validate()?;
        let text =
            serde_json::to_string(&manifest).map_err(|e| format!("cannot encode manifest: {e}"))?;
        Ok(LlmResponse {
            text,
            tokens: TokenUsage::default(),
        })
    }
}

/// Read a template tree into manifest form, replacing `{{crate_name}}`.
/// Entries are sorted so the prompt and the rendered project are stable.
pub fn read_template_files(
    templates: &Path,
    crate_name: &str,
) -> Result<BTreeMap<String, String>, String> {
    if !templates.is_dir() {
        return Err(format!(
            "template directory not found: {} (set EVENGATE_TEMPLATES)",
            templates.display()
        ));
    }
    let mut files = BTreeMap::new();
    collect_dir(templates, Path::new(""), crate_name, &mut files)?;
    Ok(files)
}

fn collect_dir(
    dir: &Path,
    rel: &Path,
    crate_name: &str,
    files: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut entries = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let child = rel.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        if file_type.is_dir() {
            collect_dir(&path, &child, crate_name, files)?;
        } else if file_type.is_file() {
            let text = fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let key = child.to_string_lossy().replace('\\', "/");
            files.insert(key, text.replace("{{crate_name}}", crate_name));
        } else {
            return Err(format!(
                "template entry is not a file or directory: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
struct RoleModels {
    default: Option<String>,
    planner: Option<String>,
    executor: Option<String>,
    reviewer: Option<String>,
}

impl RoleModels {
    fn from_env() -> Self {
        let read = |name: &str| {
            env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        RoleModels {
            default: read("EVENGATE_MODEL"),
            planner: read("EVENGATE_MODEL_PLANNER"),
            executor: read("EVENGATE_MODEL_EXECUTOR"),
            reviewer: read("EVENGATE_MODEL_REVIEWER"),
        }
    }

    fn for_role(&self, role: AgentRole) -> Option<&str> {
        let specific = match role {
            AgentRole::Planner => &self.planner,
            AgentRole::Executor => &self.executor,
            AgentRole::Reviewer => &self.reviewer,
        };
        specific.as_deref().or(self.default.as_deref())
    }
}

/// OpenAI-compatible blocking client. Endpoint, key, model, and timeout come
/// from the environment, so changing providers is configuration.
pub struct LiveClient {
    url: String,
    api_key: Option<String>,
    models: RoleModels,
    timeout: Duration,
    max_tokens: Option<u64>,
}

impl LiveClient {
    pub fn from_env() -> Result<Self, String> {
        let url = env::var("EVENGATE_LLM_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or("EVENGATE_LLM=live requires EVENGATE_LLM_URL")?;
        let api_key = env::var("EVENGATE_LLM_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let timeout = env::var("EVENGATE_LLM_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(120));
        let max_tokens = env::var("EVENGATE_LLM_MAX_TOKENS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        Ok(LiveClient {
            url,
            api_key,
            models: RoleModels::from_env(),
            timeout,
            max_tokens,
        })
    }
}

impl LlmClient for LiveClient {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, String> {
        let role = request.role.slug();
        let model = self.models.for_role(request.role).ok_or_else(|| {
            format!(
                "no model configured for role `{role}`; set EVENGATE_MODEL_{} or EVENGATE_MODEL",
                role.to_ascii_uppercase()
            )
        })?;
        let body = ChatRequest {
            model,
            messages: vec![
                ChatMessageIn {
                    role: "system",
                    content: SYSTEM_PROMPT,
                },
                ChatMessageIn {
                    role: "user",
                    content: &request.prompt,
                },
            ],
            max_tokens: self.max_tokens,
        };
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .http_status_as_error(false)
            .build()
            .new_agent();
        let mut call = agent.post(&self.url);
        if let Some(key) = &self.api_key {
            call = call.header("Authorization", format!("Bearer {key}"));
        }
        let response = call
            .send_json(&body)
            .map_err(|e| format!("LLM request to {} failed: {e}", self.url))?;
        let status = response.status().as_u16();
        let mut body = response.into_body();
        let text = body
            .read_to_string()
            .map_err(|e| format!("cannot read LLM response: {e}"))?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "LLM endpoint returned {status}: {}",
                snippet(&text)
            ));
        }
        let parsed: ChatResponse = serde_json::from_str(&text).map_err(|e| {
            format!(
                "LLM response is not OpenAI-compatible JSON: {e}: {}",
                snippet(&text)
            )
        })?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .filter(|content| !content.trim().is_empty())
            .ok_or("LLM response contained no message content")?;
        let tokens = parsed
            .usage
            .map(|usage| TokenUsage {
                input: usage.prompt_tokens,
                output: usage.completion_tokens,
            })
            .unwrap_or_default();
        Ok(LlmResponse {
            text: content,
            tokens,
        })
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessageIn<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
}

#[derive(Serialize)]
struct ChatMessageIn<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatUsage {
    #[serde(default, alias = "input_tokens")]
    prompt_tokens: u64,
    #[serde(default, alias = "output_tokens")]
    completion_tokens: u64,
}

fn snippet(text: &str) -> String {
    let trimmed = text.trim();
    let mut out: String = trimmed.chars().take(300).collect();
    if trimmed.chars().count() > 300 {
        out.push_str("...");
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedResponse {
    role: AgentRole,
    prompt_hash: String,
    response: String,
    tokens: TokenUsage,
}

/// Replays `AgentResponded` events in log order. A prompt-hash mismatch means
/// the inputs changed since the response was recorded, so the replay refuses
/// instead of silently using an answer to a different question.
#[derive(Debug, Default)]
pub struct RecordedClient {
    recorded: Vec<RecordedResponse>,
    cursor: Cell<usize>,
}

impl RecordedClient {
    pub fn from_events(events: &[Event]) -> Self {
        let mut recorded = Vec::new();
        let mut pending: Option<(AgentRole, String)> = None;
        for event in events {
            match event {
                Event::AgentInvoked { role, prompt_hash } => {
                    pending = Some((*role, prompt_hash.clone()));
                }
                Event::AgentResponded {
                    role,
                    response,
                    tokens,
                } => {
                    if let Some((invoked, prompt_hash)) = pending.take() {
                        if invoked == *role {
                            recorded.push(RecordedResponse {
                                role: *role,
                                prompt_hash,
                                response: response.clone(),
                                tokens: *tokens,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        RecordedClient {
            recorded,
            cursor: Cell::new(0),
        }
    }
}

impl LlmClient for RecordedClient {
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, String> {
        let index = self.cursor.get();
        let recorded = self.recorded.get(index).ok_or_else(|| {
            format!(
                "recorded client has no response for invocation {}; the event log does not cover this prompt",
                index + 1
            )
        })?;
        let expected = request.prompt_hash();
        if recorded.role != request.role || recorded.prompt_hash != expected {
            return Err(format!(
                "recorded response {} does not match this invocation (role {}, prompt {expected}; recorded role {}, prompt {})",
                index + 1,
                request.role.slug(),
                recorded.role.slug(),
                recorded.prompt_hash
            ));
        }
        self.cursor.set(index + 1);
        Ok(LlmResponse {
            text: recorded.response.clone(),
            tokens: recorded.tokens,
        })
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn request(prompt: &str) -> LlmRequest {
        LlmRequest {
            role: AgentRole::Executor,
            prompt: prompt.to_string(),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("evengate-llm-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn prompt_hash_is_stable_and_sensitive() {
        let a = request("build hello");
        assert_eq!(a.prompt_hash(), request("build hello").prompt_hash());
        assert_ne!(a.prompt_hash(), request("build other").prompt_hash());
        assert_eq!(a.prompt_hash().len(), 16);
    }

    #[test]
    fn manifest_parses_plain_and_fenced_json() {
        let plain = r#"{"files":{"src/main.rs":"fn main() {}\n"}}"#;
        let fenced = format!("```json\n{plain}\n```");
        assert_eq!(
            FileManifest::parse(plain).unwrap(),
            FileManifest::parse(&fenced).unwrap()
        );
        assert_eq!(
            FileManifest::parse(plain).unwrap().files["src/main.rs"],
            "fn main() {}\n"
        );
    }

    #[test]
    fn manifest_rejects_unsafe_paths() {
        for path in ["../escape", "/etc/passwd", "C:/win", "a\\b", "a/./b", ""] {
            let json = serde_json::json!({ "files": { path: "x" } }).to_string();
            assert!(
                FileManifest::parse(&json).is_err(),
                "path {path:?} should be rejected"
            );
        }
    }

    #[test]
    fn manifest_rejects_empty_and_oversized_output() {
        assert!(FileManifest::parse(r#"{"files":{}}"#).is_err());
        let big = "x".repeat(MAX_FILE_BYTES + 1);
        let json = serde_json::json!({ "files": { "big.txt": big } }).to_string();
        assert!(FileManifest::parse(&json).is_err());
    }

    #[test]
    fn template_client_renders_a_manifest() {
        let root = temp_dir("template");
        let template = root.join("template");
        fs::create_dir_all(template.join("src")).unwrap();
        fs::write(template.join("Cargo.toml"), "name = \"{{crate_name}}\"\n").unwrap();
        fs::write(template.join("src/main.rs"), "// {{crate_name}}\n").unwrap();

        let response = TemplateClient::new(template.clone(), "hello".into())
            .complete(&request("anything"))
            .unwrap();
        let manifest = FileManifest::parse(&response.text).unwrap();
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.files["Cargo.toml"], "name = \"hello\"\n");
        assert_eq!(manifest.files["src/main.rs"], "// hello\n");
        assert_eq!(response.tokens, TokenUsage::default());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn template_client_errors_when_the_directory_is_missing() {
        let root = temp_dir("missing-template");
        let error = TemplateClient::new(root.join("absent"), "hello".into())
            .complete(&request("anything"))
            .unwrap_err();
        assert!(error.contains("template directory not found"));
    }

    #[test]
    fn recorded_client_replays_in_order() {
        let prompt = request("hello");
        let events = vec![
            Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: prompt.prompt_hash(),
            },
            Event::AgentResponded {
                role: AgentRole::Executor,
                response: "first".into(),
                tokens: TokenUsage { input: 1, output: 2 },
            },
            Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: request("second").prompt_hash(),
            },
            Event::AgentResponded {
                role: AgentRole::Executor,
                response: "second".into(),
                tokens: TokenUsage::default(),
            },
        ];
        let client = RecordedClient::from_events(&events);
        let first = client.complete(&prompt).unwrap();
        assert_eq!(first.text, "first");
        assert_eq!(first.tokens, TokenUsage { input: 1, output: 2 });
        assert_eq!(client.complete(&request("second")).unwrap().text, "second");
        assert!(client.complete(&request("third")).is_err());
    }

    #[test]
    fn recorded_client_refuses_a_changed_prompt() {
        let recorded = request("original");
        let events = vec![
            Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: recorded.prompt_hash(),
            },
            Event::AgentResponded {
                role: AgentRole::Executor,
                response: "answer".into(),
                tokens: TokenUsage::default(),
            },
        ];
        let error = RecordedClient::from_events(&events)
            .complete(&request("changed"))
            .unwrap_err();
        assert!(error.contains("does not match"));
    }

    fn stub_server(status: &'static str, payload: String) -> (String, thread::JoinHandle<Option<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(
                line.starts_with("POST /chat/completions "),
                "unexpected request line: {line:?}"
            );
            let mut content_length = 0usize;
            let mut authorization = None;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap() == 0 {
                    break;
                }
                if header == "\r\n" || header == "\n" {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.trim().eq_ignore_ascii_case("content-length") {
                        content_length = value.trim().parse().unwrap();
                    }
                    if name.trim().eq_ignore_ascii_case("authorization") {
                        authorization = Some(value.trim().to_string());
                    }
                }
            }
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).unwrap();
            let sent: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(sent["model"], "test-model");
            assert_eq!(sent["messages"][0]["role"], "system");
            assert_eq!(sent["messages"][1]["content"], "hello prompt");
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
            authorization
        });
        (format!("http://{addr}/chat/completions"), handle)
    }

    fn live_client(url: String) -> LiveClient {
        LiveClient {
            url,
            api_key: Some("secret".into()),
            models: RoleModels {
                default: None,
                planner: None,
                executor: Some("test-model".into()),
                reviewer: None,
            },
            timeout: Duration::from_secs(5),
            max_tokens: None,
        }
    }

    #[test]
    fn live_client_posts_openai_compatible_json() {
        let payload = serde_json::json!({
            "choices": [{ "message": { "content": "{\"files\":{\"a.txt\":\"hi\"}}" } }],
            "usage": { "prompt_tokens": 3, "completion_tokens": 4 }
        })
        .to_string();
        let (url, server) = stub_server("200 OK", payload);
        let response = live_client(url).complete(&request("hello prompt")).unwrap();
        assert_eq!(response.text, "{\"files\":{\"a.txt\":\"hi\"}}");
        assert_eq!(response.tokens, TokenUsage { input: 3, output: 4 });
        assert_eq!(server.join().unwrap().as_deref(), Some("Bearer secret"));
    }

    #[test]
    fn live_client_reports_http_errors_with_the_body() {
        let (url, server) = stub_server(
            "500 Internal Server Error",
            r#"{"error":"provider exploded"}"#.to_string(),
        );
        let error = live_client(url).complete(&request("hello prompt")).unwrap_err();
        assert!(error.contains("500"), "{error}");
        assert!(error.contains("provider exploded"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn live_client_requires_a_model_for_the_role() {
        let client = LiveClient {
            url: "http://127.0.0.1:1/chat/completions".into(),
            api_key: None,
            models: RoleModels::default(),
            timeout: Duration::from_secs(1),
            max_tokens: None,
        };
        let error = client.complete(&request("x")).unwrap_err();
        assert!(error.contains("EVENGATE_MODEL_EXECUTOR"), "{error}");
    }
}
