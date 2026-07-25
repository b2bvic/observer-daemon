use crate::rubric::{ClassificationMetadata, ContentClass};
use serde_json::Value;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// Extracted assistant message from a Claude Code session JSONL.
pub struct AssistantMessage {
    pub text: String,
    pub session_id: String,
    pub prompt: String,
    pub cwd: String,
    pub metadata: ClassificationMetadata,
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

/// Parse assistant text blocks from Claude Code session JSONL.
///
/// Claude Code sessions are append-only JSONL files. Each line is a JSON object.
/// We care about lines where type == "assistant". The message.content array
/// contains blocks; blocks with type == "text" hold the output to validate.
/// We skip "thinking" blocks, "tool_use" blocks, and "tool_result" blocks.
pub fn extract_assistant_messages(lines: &[String]) -> Vec<AssistantMessage> {
    let mut messages = Vec::new();
    let mut last_user_prompt = String::new();
    let mut current_cwd = String::new();

    for line in lines {
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        if let Some(cwd) = val.get("cwd").and_then(|c| c.as_str()) {
            current_cwd = cwd.to_string();
        }

        let msg_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");

        if matches!(msg_type, "human" | "user") {
            // Capture user prompt for layer alignment checks
            if let Some(content) = val.get("message").and_then(|m| m.get("content")) {
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
                    last_user_prompt = texts.join("\n");
                } else if let Some(s) = content.as_str() {
                    last_user_prompt = s.to_string();
                }
            }
        }

        if msg_type != "assistant" {
            continue;
        }

        let session_id = val
            .get("sessionId")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        let content = match val.get("message").and_then(|m| m.get("content")) {
            Some(c) => c,
            None => continue,
        };

        let blocks = match content.as_array() {
            Some(arr) => arr,
            None => continue,
        };

        let mut text_parts = Vec::new();
        for block in blocks {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if block_type == "text" {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    text_parts.push(text);
                }
            }
            // Skip: thinking, tool_use, tool_result
        }

        if text_parts.is_empty() {
            continue;
        }

        messages.push(AssistantMessage {
            text: text_parts.join("\n"),
            session_id,
            prompt: last_user_prompt.clone(),
            cwd: current_cwd.clone(),
            metadata: classification_metadata(&val),
        });
    }

    messages
}

/// Parse assistant text blocks from Codex CLI session JSONL.
///
/// Codex records session events as JSONL entries with top-level
/// `type == "response_item"` and Responses-style payloads. User prompts
/// arrive as role=user messages; assistant output arrives as role=assistant
/// messages with `output_text` content blocks.
pub fn extract_codex_messages(lines: &[String], session_id: &str) -> Vec<AssistantMessage> {
    let mut messages = Vec::new();
    let mut last_user_prompt = String::new();
    let mut current_cwd = String::new();

    for line in lines {
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        if val.get("type").and_then(|t| t.as_str()) == Some("session_meta") {
            if let Some(cwd) = val
                .get("payload")
                .and_then(|p| p.get("cwd"))
                .and_then(|c| c.as_str())
            {
                current_cwd = cwd.to_string();
            }
            continue;
        }

        if val.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            continue;
        }

        let Some(payload) = val.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }

        let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let content = match payload.get("content").and_then(|c| c.as_array()) {
            Some(c) => c,
            None => continue,
        };

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
            continue;
        }

        let text = text_parts.join("\n");
        if role == "user" {
            last_user_prompt = text;
            continue;
        }
        if role != "assistant" {
            continue;
        }

        messages.push(AssistantMessage {
            text,
            session_id: session_id.to_string(),
            prompt: last_user_prompt.clone(),
            cwd: current_cwd.clone(),
            metadata: classification_metadata(payload),
        });
    }

    messages
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

/// Read new lines from a JSONL file starting at the given byte offset.
/// Returns the new lines and the updated offset.
pub fn read_new_lines(path: &Path, offset: u64) -> std::io::Result<(Vec<String>, u64)> {
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    let file_len = metadata.len();

    let start_offset = if file_len < offset { 0 } else { offset };

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

        new_offset += bytes_read as u64;
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end_matches(['\r', '\n']);
        if !line.trim().is_empty() {
            lines.push(line.to_string());
        }
    }

    Ok((lines, new_offset))
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
        codex_session_id_from_path, extract_assistant_messages, extract_codex_messages,
        extract_markdown_exchanges,
    };
    use std::path::Path;

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
}
