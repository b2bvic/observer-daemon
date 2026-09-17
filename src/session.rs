use crate::rubric::{ClassificationMetadata, ContentClass};
use serde_json::Value;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Extracted assistant message from a Claude Code session JSONL.
pub struct AssistantMessage {
    pub text: String,
    pub session_id: String,
    pub prompt: String,
    pub cwd: String,
    pub metadata: ClassificationMetadata,
}

/// Recognized native JSONL transcript formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonlProvider {
    Claude,
    Codex,
}

/// Per-file parser state retained across JSONL read batches.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JsonlParserState {
    pub last_user_prompt: String,
    pub cwd: String,
    pub native_session_id: String,
    pub provider: Option<JsonlProvider>,
    file_identity: Option<(u64, u64)>,
}

/// Complete JSONL lines read from a watched file.
pub struct JsonlRead {
    pub lines: Vec<String>,
    pub new_offset: u64,
    /// True when the file was shorter than the retained offset.
    pub reset: bool,
}

/// Messages ingested from a JSONL file batch.
pub struct JsonlIngest {
    pub messages: Vec<AssistantMessage>,
    pub new_offset: u64,
}

fn classification_metadata(value: &Value) -> ClassificationMetadata {
    let class_value = value
        .get("content_class")
        .or_else(|| value.pointer("/metadata/content_class"))
        .or_else(|| value.pointer("/payload/metadata/content_class"))
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse::<ContentClass>().ok());
    let labels = value
        .get("labels")
        .or_else(|| value.pointer("/metadata/labels"))
        .or_else(|| value.pointer("/payload/metadata/labels"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    ClassificationMetadata {
        content_class: class_value,
        labels,
    }
}

/// Classify a JSONL record from its contents, not its path.
pub fn detect_jsonl_provider(value: &Value) -> Option<JsonlProvider> {
    let msg_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    match msg_type {
        "session_meta" | "response_item" | "event_msg" | "turn_context" => {
            Some(JsonlProvider::Codex)
        }
        "assistant" | "user" | "human" => Some(JsonlProvider::Claude),
        _ => None,
    }
}

pub fn jsonl_source(state: &JsonlParserState) -> &'static str {
    match state.provider {
        Some(JsonlProvider::Codex) => "responses-jsonl",
        _ => "message-jsonl",
    }
}

fn claude_user_prompt(val: &Value) -> Option<String> {
    let content = val.get("message")?.get("content")?;
    if let Some(arr) = content.as_array() {
        let texts: Vec<&str> = arr
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    block.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect();
        if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n"))
        }
    } else {
        content.as_str().map(ToString::to_string)
    }
}

fn claude_assistant_text(val: &Value) -> Option<String> {
    let blocks = val.get("message")?.get("content")?.as_array()?;
    let mut text_parts = Vec::new();
    for block in blocks {
        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if block_type == "text" {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                text_parts.push(text);
            }
        }
    }
    if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    }
}

fn ingest_claude_line(val: &Value, state: &mut JsonlParserState) -> Option<AssistantMessage> {
    if let Some(id) = val
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        if !state.native_session_id.is_empty() && state.native_session_id != id {
            state.last_user_prompt.clear();
            state.cwd.clear();
        }
        state.native_session_id = id.to_string();
    }
    if let Some(cwd) = val.get("cwd").and_then(|c| c.as_str()) {
        state.cwd = cwd.to_string();
    }

    let msg_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");

    if matches!(msg_type, "human" | "user") {
        if let Some(prompt) = claude_user_prompt(val) {
            state.last_user_prompt = prompt;
        }
        return None;
    }

    if msg_type != "assistant" {
        return None;
    }

    let session_id = state.native_session_id.clone();
    if !session_id.is_empty() {
        state.native_session_id = session_id.clone();
    }

    Some(AssistantMessage {
        text: claude_assistant_text(val)?,
        session_id,
        prompt: state.last_user_prompt.clone(),
        cwd: state.cwd.clone(),
        metadata: classification_metadata(val),
    })
}

fn ingest_codex_line(val: &Value, state: &mut JsonlParserState) -> Option<AssistantMessage> {
    if val.get("type").and_then(|t| t.as_str()) == Some("session_meta") {
        if let Some(payload) = val.get("payload") {
            if let Some(cwd) = payload.get("cwd").and_then(|c| c.as_str()) {
                state.cwd = cwd.to_string();
            }
            if let Some(id) = payload.get("id").and_then(|c| c.as_str()) {
                if !id.is_empty() {
                    state.native_session_id = id.to_string();
                }
            }
        }
        return None;
    }

    if val.get("type").and_then(|t| t.as_str()) != Some("response_item") {
        return None;
    }

    let payload = val.get("payload")?;
    if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
        return None;
    }

    let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
    let content = payload.get("content")?.as_array()?;

    let mut text_parts = Vec::new();
    for block in content {
        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if matches!(block_type, "input_text" | "output_text" | "text") {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                text_parts.push(text);
            }
        }
    }

    if text_parts.is_empty() {
        return None;
    }

    let text = text_parts.join("\n");
    if role == "user" {
        state.last_user_prompt = text;
        return None;
    }
    if role != "assistant" {
        return None;
    }

    Some(AssistantMessage {
        text,
        session_id: state.native_session_id.clone(),
        prompt: state.last_user_prompt.clone(),
        cwd: state.cwd.clone(),
        metadata: classification_metadata(payload),
    })
}

/// Ingest complete JSONL lines, updating per-file parser state.
pub fn ingest_jsonl_lines(lines: &[String], state: &mut JsonlParserState) -> Vec<AssistantMessage> {
    let mut messages = Vec::new();
    for line in lines {
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if state.provider.is_none() {
            state.provider = detect_jsonl_provider(&val);
        }
        let Some(provider) = state.provider else {
            continue;
        };
        let message = match provider {
            JsonlProvider::Claude => ingest_claude_line(&val, state),
            JsonlProvider::Codex => ingest_codex_line(&val, state),
        };
        if let Some(message) = message {
            messages.push(message);
        }
    }
    messages
}

/// Parse assistant text blocks from Claude Code session JSONL.
///
/// Claude Code sessions are append-only JSONL files. Each line is a JSON object.
/// We care about lines where type == "assistant". The message.content array
/// contains blocks; blocks with type == "text" hold the output to validate.
/// We skip "thinking" blocks, "tool_use" blocks, and "tool_result" blocks.
pub fn extract_assistant_messages(lines: &[String]) -> Vec<AssistantMessage> {
    let mut state = JsonlParserState {
        provider: Some(JsonlProvider::Claude),
        ..Default::default()
    };
    ingest_jsonl_lines(lines, &mut state)
}

/// Parse assistant text blocks from Codex CLI session JSONL.
///
/// Codex records session events as JSONL entries with top-level
/// `type == "response_item"` and Responses-style payloads. User prompts
/// arrive as role=user messages; assistant output arrives as role=assistant
/// messages with `output_text` content blocks.
pub fn extract_codex_messages(lines: &[String], session_id: &str) -> Vec<AssistantMessage> {
    let mut state = JsonlParserState {
        native_session_id: session_id.to_string(),
        provider: Some(JsonlProvider::Codex),
        ..Default::default()
    };
    ingest_jsonl_lines(lines, &mut state)
}

pub fn codex_session_id_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    match stem.rsplit_once("-019") {
        Some((_, tail)) => format!("019{}", tail),
        None => stem,
    }
}

/// Read new complete JSONL lines starting at `offset`.
///
/// A trailing fragment without a newline is left unconsumed. If the file is
/// shorter than `offset`, reading starts at byte 0 and `reset` is true.
pub fn read_new_jsonl(path: &Path, offset: u64) -> std::io::Result<JsonlRead> {
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    let file_len = metadata.len();
    let reset = file_len < offset;
    let start_offset = if reset { 0 } else { offset };

    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(start_offset))?;

    let mut lines = Vec::new();
    let mut new_offset = start_offset;
    let mut buf = Vec::new();

    loop {
        buf.clear();
        let bytes_read = reader.read_until(b'\n', &mut buf)?;
        if bytes_read == 0 {
            break;
        }
        if !buf.ends_with(&[b'\n']) {
            break;
        }

        new_offset += bytes_read as u64;
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end_matches(['\r', '\n']);
        if !line.trim().is_empty() {
            lines.push(line.to_string());
        }
    }

    Ok(JsonlRead {
        lines,
        new_offset,
        reset,
    })
}

/// Read new lines from a JSONL file starting at the given byte offset.
/// Returns the new lines and the updated offset.
pub fn read_new_lines(path: &Path, offset: u64) -> std::io::Result<(Vec<String>, u64)> {
    let read = read_new_jsonl(path, offset)?;
    Ok((read.lines, read.new_offset))
}

/// Ingest newly appended complete JSONL lines from a watched file.
///
/// Resets `state` when the file is shorter than `offset`. Codex session id
/// falls back to the path stem only when `session_meta` payload.id is absent.
pub fn ingest_jsonl_file(
    path: &Path,
    offset: u64,
    state: &mut JsonlParserState,
) -> std::io::Result<JsonlIngest> {
    let metadata = std::fs::metadata(path)?;
    let identity = (metadata.dev(), metadata.ino());
    let replaced = state.file_identity.is_some_and(|prior| prior != identity);
    let read = read_new_jsonl(path, if replaced { 0 } else { offset })?;
    if read.reset || replaced {
        *state = JsonlParserState::default();
    }
    state.file_identity = Some(identity);
    let mut messages = ingest_jsonl_lines(&read.lines, state);
    if state.provider == Some(JsonlProvider::Codex) && state.native_session_id.is_empty() {
        let fallback = codex_session_id_from_path(path);
        if !fallback.is_empty() {
            state.native_session_id = fallback.clone();
            for message in &mut messages {
                if message.session_id.is_empty() {
                    message.session_id = fallback.clone();
                }
            }
        }
    }
    Ok(JsonlIngest {
        messages,
        new_offset: read.new_offset,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct MarkdownExchange {
    pub prompt: String,
    pub response: String,
}

/// Extract prompt-response pairs from an append-only Markdown conversation.
/// The prompt is carried into validation instead of being replaced by an empty
/// string.
pub fn extract_markdown_exchanges(content: &str, after_offset: usize) -> Vec<MarkdownExchange> {
    let slice = if after_offset < content.len() {
        &content[after_offset..]
    } else {
        return Vec::new();
    };

    let mut exchanges = Vec::new();
    let mut prompt = String::new();
    let mut in_response = false;
    let mut response = String::new();

    for line in slice.lines() {
        if let Some(value) = line.strip_prefix("**User:**") {
            if in_response && !response.trim().is_empty() {
                exchanges.push(MarkdownExchange {
                    prompt: prompt.trim().to_string(),
                    response: response.trim().to_string(),
                });
            }
            prompt = value.trim().to_string();
            response.clear();
            in_response = false;
        } else if let Some(value) = line.strip_prefix("**Assistant:**") {
            if in_response && !response.trim().is_empty() {
                exchanges.push(MarkdownExchange {
                    prompt: prompt.trim().to_string(),
                    response: response.trim().to_string(),
                });
            }
            response = value.trim().to_string();
            in_response = true;
        } else if in_response {
            if line.starts_with("---") {
                exchanges.push(MarkdownExchange {
                    prompt: prompt.trim().to_string(),
                    response: response.trim().to_string(),
                });
                response.clear();
                in_response = false;
            } else {
                response.push('\n');
                response.push_str(line);
            }
        } else if !prompt.is_empty() && !line.starts_with("---") {
            prompt.push('\n');
            prompt.push_str(line);
        }
    }

    if in_response && !response.trim().is_empty() {
        exchanges.push(MarkdownExchange {
            prompt: prompt.trim().to_string(),
            response: response.trim().to_string(),
        });
    }

    exchanges
}

#[cfg(test)]
mod tests {
    use super::{
        JsonlParserState, JsonlProvider, codex_session_id_from_path, extract_assistant_messages,
        extract_codex_messages, extract_markdown_exchanges, ingest_jsonl_file, jsonl_source,
    };
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir(label: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("observer-daemon-tests")
            .join(format!(
                "{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("time")
                    .as_nanos()
            ));
        std::fs::create_dir_all(&dir).expect("test dir");
        dir
    }

    fn append_line(path: &Path, line: &str) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("append");
        writeln!(file, "{line}").expect("write line");
    }

    fn append_bytes(path: &Path, bytes: &[u8]) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("append");
        file.write_all(bytes).expect("write bytes");
    }

    #[test]
    fn extracts_codex_assistant_message_and_prompt() {
        let lines = vec![
            r#"{"type":"session_meta","payload":{"cwd":"/workspace/example"}}"#.to_string(),
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"implement it"}]}}"#.to_string(),
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"implemented"}]}}"#.to_string(),
        ];

        let messages = extract_codex_messages(&lines, "codex-session");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "codex-session");
        assert_eq!(messages[0].prompt, "implement it");
        assert_eq!(messages[0].text, "implemented");
        assert_eq!(messages[0].cwd, "/workspace/example");
    }

    #[test]
    fn extracts_claude_assistant_message_prompt_and_cwd() {
        let lines = vec![
            r#"{"type":"user","cwd":"/workspace/example","message":{"content":[{"type":"text","text":"review it"}]}}"#.to_string(),
            r#"{"type":"assistant","sessionId":"message-session","cwd":"/workspace/example","message":{"content":[{"type":"text","text":"reviewed"}]}}"#.to_string(),
        ];

        let messages = extract_assistant_messages(&lines);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "message-session");
        assert_eq!(messages[0].prompt, "review it");
        assert_eq!(messages[0].text, "reviewed");
        assert_eq!(messages[0].cwd, "/workspace/example");
    }

    #[test]
    fn derives_codex_session_id_from_rollout_path() {
        let path = Path::new(
            "/workspace/sessions/2026/05/05/rollout-2026-05-05T19-56-41-019dfa92-8a7b-7860-87d1-8a8939cec3fe.jsonl",
        );

        assert_eq!(
            codex_session_id_from_path(path),
            "019dfa92-8a7b-7860-87d1-8a8939cec3fe"
        );
    }

    #[test]
    fn markdown_exchange_preserves_prompt_for_validation() {
        let content = concat!(
            "**User:** Help me prepare a career briefing.\n",
            "**Assistant:** Lead with measurable experience.\n",
            "---\n",
        );

        let exchanges = extract_markdown_exchanges(content, 0);

        assert_eq!(exchanges.len(), 1);
        assert_eq!(exchanges[0].prompt, "Help me prepare a career briefing.");
        assert_eq!(exchanges[0].response, "Lead with measurable experience.");
    }

    #[test]
    fn split_prompt_assistant_batches_retain_state_for_both_providers() {
        let dir = test_dir("split-batches");
        let claude_path = dir.join("claude.jsonl");
        let codex_path = dir.join("codex.jsonl");

        append_line(
            &claude_path,
            r#"{"type":"user","cwd":"/claude/work","message":{"content":[{"type":"text","text":"claude prompt"}]}}"#,
        );
        let mut claude_state = JsonlParserState::default();
        let first_claude =
            ingest_jsonl_file(&claude_path, 0, &mut claude_state).expect("claude user");
        assert!(first_claude.messages.is_empty());
        assert_eq!(claude_state.last_user_prompt, "claude prompt");
        assert_eq!(claude_state.cwd, "/claude/work");
        assert_eq!(claude_state.provider, Some(JsonlProvider::Claude));

        append_line(
            &claude_path,
            r#"{"type":"assistant","sessionId":"claude-1","cwd":"/claude/work","message":{"content":[{"type":"text","text":"claude reply"}]}}"#,
        );
        let second_claude =
            ingest_jsonl_file(&claude_path, first_claude.new_offset, &mut claude_state)
                .expect("claude assistant");
        assert_eq!(second_claude.messages.len(), 1);
        assert_eq!(second_claude.messages[0].prompt, "claude prompt");
        assert_eq!(second_claude.messages[0].text, "claude reply");
        assert_eq!(second_claude.messages[0].cwd, "/claude/work");
        assert_eq!(second_claude.messages[0].session_id, "claude-1");

        append_line(
            &codex_path,
            r#"{"type":"session_meta","payload":{"id":"codex-native-id","cwd":"/codex/work"}}"#,
        );
        append_line(
            &codex_path,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex prompt"}]}}"#,
        );
        let mut codex_state = JsonlParserState::default();
        let first_codex = ingest_jsonl_file(&codex_path, 0, &mut codex_state).expect("codex user");
        assert!(first_codex.messages.is_empty());
        assert_eq!(codex_state.last_user_prompt, "codex prompt");
        assert_eq!(codex_state.cwd, "/codex/work");
        assert_eq!(codex_state.native_session_id, "codex-native-id");
        assert_eq!(codex_state.provider, Some(JsonlProvider::Codex));

        append_line(
            &codex_path,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"codex reply"}]}}"#,
        );
        let second_codex = ingest_jsonl_file(&codex_path, first_codex.new_offset, &mut codex_state)
            .expect("codex assistant");
        assert_eq!(second_codex.messages.len(), 1);
        assert_eq!(second_codex.messages[0].prompt, "codex prompt");
        assert_eq!(second_codex.messages[0].text, "codex reply");
        assert_eq!(second_codex.messages[0].cwd, "/codex/work");
        assert_eq!(second_codex.messages[0].session_id, "codex-native-id");
        assert_eq!(jsonl_source(&codex_state), "responses-jsonl");
    }

    #[test]
    fn two_files_do_not_share_parser_state() {
        let dir = test_dir("two-files");
        let path_a = dir.join("a.jsonl");
        let path_b = dir.join("b.jsonl");
        let mut state_a = JsonlParserState::default();
        let mut state_b = JsonlParserState::default();

        append_line(
            &path_a,
            r#"{"type":"user","cwd":"/a","message":{"content":[{"type":"text","text":"prompt a"}]}}"#,
        );
        append_line(
            &path_b,
            r#"{"type":"user","cwd":"/b","message":{"content":[{"type":"text","text":"prompt b"}]}}"#,
        );
        let first_a = ingest_jsonl_file(&path_a, 0, &mut state_a).expect("a user");
        let first_b = ingest_jsonl_file(&path_b, 0, &mut state_b).expect("b user");
        assert_eq!(state_a.last_user_prompt, "prompt a");
        assert_eq!(state_b.last_user_prompt, "prompt b");

        append_line(
            &path_a,
            r#"{"type":"assistant","sessionId":"sess-a","message":{"content":[{"type":"text","text":"reply a"}]}}"#,
        );
        append_line(
            &path_b,
            r#"{"type":"assistant","sessionId":"sess-b","message":{"content":[{"type":"text","text":"reply b"}]}}"#,
        );
        let second_a =
            ingest_jsonl_file(&path_a, first_a.new_offset, &mut state_a).expect("a assistant");
        let second_b =
            ingest_jsonl_file(&path_b, first_b.new_offset, &mut state_b).expect("b assistant");

        assert_eq!(second_a.messages[0].prompt, "prompt a");
        assert_eq!(second_a.messages[0].cwd, "/a");
        assert_eq!(second_a.messages[0].session_id, "sess-a");
        assert_eq!(second_b.messages[0].prompt, "prompt b");
        assert_eq!(second_b.messages[0].cwd, "/b");
        assert_eq!(second_b.messages[0].session_id, "sess-b");
        assert_ne!(state_a.last_user_prompt, state_b.last_user_prompt);
    }

    #[test]
    fn renamed_codex_export_is_recognized_from_content() {
        let dir = test_dir("renamed-codex");
        let path = dir.join("archive").join("export-renamed.jsonl");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("archive dir");
        assert!(
            !path.to_string_lossy().contains("/.codex/sessions/"),
            "fixture path must not include the old path detector"
        );

        append_line(
            &path,
            r#"{"type":"session_meta","payload":{"id":"019abcde-1111-2222-3333-444444444444","cwd":"/archived/project"}}"#,
        );
        append_line(
            &path,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"archived prompt"}]}}"#,
        );
        append_line(
            &path,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"archived reply"}]}}"#,
        );

        let mut state = JsonlParserState::default();
        let ingested = ingest_jsonl_file(&path, 0, &mut state).expect("renamed codex");
        assert_eq!(state.provider, Some(JsonlProvider::Codex));
        assert_eq!(jsonl_source(&state), "responses-jsonl");
        assert_eq!(ingested.messages.len(), 1);
        assert_eq!(
            ingested.messages[0].session_id,
            "019abcde-1111-2222-3333-444444444444"
        );
        assert_eq!(ingested.messages[0].prompt, "archived prompt");
        assert_eq!(ingested.messages[0].text, "archived reply");
        assert_eq!(ingested.messages[0].cwd, "/archived/project");
    }

    #[test]
    fn truncation_resets_parser_state() {
        let dir = test_dir("truncation");
        let path = dir.join("session.jsonl");
        let mut state = JsonlParserState::default();

        append_line(
            &path,
            r#"{"type":"user","cwd":"/old","message":{"content":[{"type":"text","text":"old prompt that makes the first file longer"}]}}"#,
        );
        append_line(
            &path,
            r#"{"type":"assistant","sessionId":"old-session","message":{"content":[{"type":"text","text":"old reply"}]}}"#,
        );
        let first = ingest_jsonl_file(&path, 0, &mut state).expect("first file");
        assert_eq!(
            first.messages[0].prompt,
            "old prompt that makes the first file longer"
        );
        assert_eq!(
            state.last_user_prompt,
            "old prompt that makes the first file longer"
        );
        assert!(first.new_offset > 0);

        std::fs::write(
            &path,
            concat!(
                r#"{"type":"user","cwd":"/new","message":{"content":[{"type":"text","text":"new prompt"}]}}"#,
                "\n",
                r#"{"type":"assistant","sessionId":"new-session","message":{"content":[{"type":"text","text":"new reply"}]}}"#,
                "\n",
            ),
        )
        .expect("replace file");

        let second = ingest_jsonl_file(&path, first.new_offset, &mut state).expect("truncated");
        assert_eq!(second.messages.len(), 1);
        assert_eq!(second.messages[0].prompt, "new prompt");
        assert_eq!(second.messages[0].text, "new reply");
        assert_eq!(second.messages[0].cwd, "/new");
        assert_eq!(second.messages[0].session_id, "new-session");
        assert_eq!(state.last_user_prompt, "new prompt");
        assert_eq!(state.cwd, "/new");
        assert_eq!(state.native_session_id, "new-session");
    }

    #[test]
    fn partial_line_is_not_consumed_until_newline() {
        let dir = test_dir("partial-line");
        let path = dir.join("session.jsonl");
        let mut state = JsonlParserState::default();

        let user =
            r#"{"type":"user","message":{"content":[{"type":"text","text":"partial prompt"}]}}"#;
        append_line(&path, user);
        let incomplete = r#"{"type":"assistant","sessionId":"partial-1","message":{"content":[{"type":"text","text":"partial"#;
        append_bytes(&path, incomplete.as_bytes());

        let first = ingest_jsonl_file(&path, 0, &mut state).expect("incomplete");
        assert!(first.messages.is_empty());
        assert_eq!(first.new_offset, (user.len() + 1) as u64);
        assert_eq!(state.last_user_prompt, "partial prompt");

        append_bytes(&path, br#""}]}}"#);
        let still_incomplete =
            ingest_jsonl_file(&path, first.new_offset, &mut state).expect("still incomplete");
        assert!(still_incomplete.messages.is_empty());
        assert_eq!(still_incomplete.new_offset, first.new_offset);

        append_bytes(&path, b"\n");
        let complete =
            ingest_jsonl_file(&path, still_incomplete.new_offset, &mut state).expect("complete");
        assert_eq!(complete.messages.len(), 1);
        assert_eq!(complete.messages[0].prompt, "partial prompt");
        assert_eq!(complete.messages[0].text, "partial");
        assert_eq!(complete.messages[0].session_id, "partial-1");
    }
    #[test]
    fn replacement_with_larger_file_resets_reader_and_prompt() {
        let dir = test_dir("replacement");
        let path = dir.join("session.jsonl");
        append_line(
            &path,
            r#"{"type":"user","message":{"content":"old prompt"}}"#,
        );
        let mut state = JsonlParserState::default();
        let first = ingest_jsonl_file(&path, 0, &mut state).unwrap();
        let replacement = dir.join("replacement.jsonl");
        append_line(
            &replacement,
            r#"{"type":"assistant","sessionId":"new","message":{"content":[{"type":"text","text":"a replacement response longer than the original transcript"}]}}"#,
        );
        std::fs::rename(&replacement, &path).unwrap();
        let next = ingest_jsonl_file(&path, first.new_offset, &mut state).unwrap();
        assert_eq!(next.messages.len(), 1);
        assert_eq!(next.messages[0].prompt, "");
        assert_eq!(next.messages[0].session_id, "new");
    }

    #[test]
    fn claude_tool_results_keep_prompt_and_user_session_identity() {
        let messages = extract_assistant_messages(&[
            r#"{"type":"user","sessionId":"from-user","message":{"content":"original prompt"}}"#.into(),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"tool result"}]}}"#.into(),
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"answer"}]}}"#.into(),
        ]);
        assert_eq!(messages[0].prompt, "original prompt");
        assert_eq!(messages[0].session_id, "from-user");
    }
}
