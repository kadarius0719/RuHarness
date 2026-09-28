//! Claude Code's stream-JSON, both ways (docs/CHAT-PANE-DESIGN.md §1.3):
//! one stdout line → one [`In`], the small event set the pane knows; and
//! the lines the cockpit writes (user messages, permission answers,
//! `interrupt`, `initialize`). Shapes as verified live (the spike's logs and
//! `tests/fixtures/chat/`): the runtime prints one `assistant` line per
//! content block, echoes each user message it read with `isReplay: true` and
//! the cockpit's `uuid`, and echoes each `control_response` it received.
//! Nothing here trusts a value: every string is the model's or the
//! runtime's, filtered by the view before it is drawn.

use serde_json::{json, Value};

/// One content block of an `assistant` line.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Text the model wrote.
    Text(String),
    /// A tool call.
    ToolUse {
        /// The call's id (`toolu_…`), which its permission request and its
        /// result name.
        id: String,
        /// The tool (`mcp__harness__harness_status`, …).
        name: String,
        /// Its input.
        input: Value,
    },
    /// Thinking (never shown).
    Thinking,
    /// A block of another type (never shown).
    Other,
}

/// A `result` line: the end of a turn.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnResult {
    /// `success`, `error_during_execution`, `error_max_turns`, …
    pub subtype: String,
    /// The runtime marked it an error.
    pub is_error: bool,
    /// Cumulative for the process, in USD (a subscription reports it too).
    pub total_cost_usd: Option<f64>,
    /// `completed`, `aborted_streaming`, `aborted_tools`, …
    pub terminal_reason: Option<String>,
    /// The error texts, when it failed.
    pub errors: Vec<String>,
    /// The final text, or an error's words.
    pub result: Option<String>,
    /// Turns still queued behind this one (absent = 0).
    pub queued_turn_count: u64,
    /// The turn's duration, in ms.
    pub duration_ms: Option<u64>,
}

/// One stdout line, read.
#[derive(Debug, Clone, PartialEq)]
pub enum In {
    /// `system/init`: a turn starts.
    Init {
        /// The model that answers (`claude-haiku-4-5-…`).
        model: Option<String>,
        /// `claude_code_version`.
        version: Option<String>,
        /// `permissionMode`.
        permission_mode: Option<String>,
        /// The tools offered, by name.
        tools: Vec<String>,
        /// The MCP servers: (name, status).
        servers: Vec<(String, String)>,
        /// `apiKeySource` (`none` under a subscription).
        api_key_source: Option<String>,
    },
    /// `stream_event` `message_start`: a new assistant message.
    MessageStart {
        /// Its id.
        id: String,
    },
    /// `stream_event` `content_block_start`: block `index` of the current
    /// message begins (the next `assistant` line is that block).
    BlockStart {
        /// The block's index in its message.
        index: u64,
    },
    /// `stream_event` `content_block_delta` `text_delta` — the message is
    /// the one the last `message_start` opened.
    TextDelta {
        /// The block's index in its message.
        index: u64,
        /// The text.
        text: String,
    },
    /// One `assistant` line: one block of message `id`.
    Assistant {
        /// The message id.
        id: String,
        /// The model that wrote it (`<synthetic>` for the runtime's own).
        model: Option<String>,
        /// Its blocks (one, as the runtime prints them).
        blocks: Vec<Block>,
        /// An `error` value (`authentication_failed`, `model_not_found`, …).
        error: Option<String>,
    },
    /// A `user` line carrying tool results (the runtime's own).
    ToolResults(Vec<ToolResult>),
    /// A `user` line of text: an echo of the cockpit's message (`isReplay`
    /// with its `uuid`), the runtime's interrupt marker, or a message the
    /// cockpit did not send.
    UserText {
        /// The message's `uuid`.
        uuid: Option<String>,
        /// `isReplay: true`.
        replay: bool,
        /// Its text blocks, joined.
        text: String,
    },
    /// `control_request` `can_use_tool`: a tool call waiting for the
    /// cockpit's answer.
    CanUseTool {
        /// The request id the answer must name.
        request_id: String,
        /// The tool's full name.
        tool: String,
        /// `mcp_server.name`, when it came from an MCP server.
        server: Option<String>,
        /// Its input.
        input: Value,
        /// The call's id.
        tool_use_id: Option<String>,
    },
    /// Any other `control_request`: answered at once with an error.
    OtherRequest {
        /// Its id.
        request_id: String,
        /// Its subtype.
        subtype: String,
    },
    /// `control_cancel_request`: the request is withdrawn.
    Cancel {
        /// The withdrawn request.
        request_id: String,
    },
    /// A `control_response` — to the cockpit's own `initialize` or
    /// `interrupt`, or the runtime's echo of an answer it received.
    Response {
        /// The request it answers.
        request_id: String,
        /// `success` or `error`.
        subtype: String,
        /// For `interrupt`: the messages it cancelled (by `uuid`).
        cancelled: Vec<String>,
        /// Its error, if any.
        error: Option<String>,
    },
    /// `command_lifecycle`: a message's state (`queued`, `started`,
    /// `cancelled`, `completed`).
    Lifecycle {
        /// The message's `uuid`.
        uuid: String,
        /// Its state.
        state: String,
    },
    /// `result`: the turn ended.
    Result(TurnResult),
    /// `rate_limit_event`.
    RateLimit {
        /// `allowed`, `allowed_warning`, `rejected`.
        status: String,
        /// When the window resets (Unix seconds).
        resets_at: Option<i64>,
        /// An error code, if any.
        error_code: Option<String>,
    },
    /// A line the pane has nothing to do with (status, thinking tokens,
    /// other stream events).
    Ignored,
    /// Not JSON, or not an object with a `type`.
    Unparsed,
}

/// One tool result of a `user` line.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    /// The call it answers.
    pub tool_use_id: String,
    /// The runtime marked it an error (a denial, a failed tool).
    pub is_error: bool,
    /// Its text (the first text part).
    pub text: String,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// The text of a content value: a string, or the text parts of an array.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn block(b: &Value) -> Block {
    match b.get("type").and_then(Value::as_str) {
        Some("text") => Block::Text(text(b, "text").unwrap_or_default()),
        Some("tool_use") => Block::ToolUse {
            id: text(b, "id").unwrap_or_default(),
            name: text(b, "name").unwrap_or_default(),
            input: b.get("input").cloned().unwrap_or(Value::Null),
        },
        Some("thinking") | Some("redacted_thinking") => Block::Thinking,
        _ => Block::Other,
    }
}

/// Read one stdout line.
pub fn parse(line: &str) -> In {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return In::Unparsed;
    };
    let Some(kind) = v.get("type").and_then(Value::as_str) else {
        return In::Unparsed;
    };
    match kind {
        "system" if v.get("subtype").and_then(Value::as_str) == Some("init") => In::Init {
            model: text(&v, "model"),
            version: text(&v, "claude_code_version"),
            permission_mode: text(&v, "permissionMode"),
            tools: v
                .get("tools")
                .and_then(Value::as_array)
                .map(|t| {
                    t.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            servers: v
                .get("mcp_servers")
                .and_then(Value::as_array)
                .map(|s| {
                    s.iter()
                        .map(|s| {
                            (
                                text(s, "name").unwrap_or_default(),
                                text(s, "status").unwrap_or_default(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            api_key_source: text(&v, "apiKeySource"),
        },
        "stream_event" => {
            let ev = v.get("event").unwrap_or(&Value::Null);
            match ev.get("type").and_then(Value::as_str) {
                Some("message_start") => match ev.pointer("/message/id").and_then(Value::as_str) {
                    Some(id) => In::MessageStart { id: id.to_string() },
                    None => In::Ignored,
                },
                Some("content_block_start") => match ev.get("index").and_then(Value::as_u64) {
                    Some(index) => In::BlockStart { index },
                    None => In::Ignored,
                },
                Some("content_block_delta") => {
                    let delta = ev.get("delta").unwrap_or(&Value::Null);
                    match (
                        delta.get("type").and_then(Value::as_str),
                        ev.get("index").and_then(Value::as_u64),
                    ) {
                        (Some("text_delta"), Some(index)) => In::TextDelta {
                            index,
                            text: text(delta, "text").unwrap_or_default(),
                        },
                        _ => In::Ignored,
                    }
                }
                _ => In::Ignored,
            }
        }
        "assistant" => {
            let msg = v.get("message").unwrap_or(&Value::Null);
            In::Assistant {
                id: text(msg, "id").unwrap_or_default(),
                model: text(msg, "model"),
                blocks: msg
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|c| c.iter().map(block).collect())
                    .unwrap_or_default(),
                error: text(&v, "error"),
            }
        }
        "user" => {
            let content = v.pointer("/message/content").unwrap_or(&Value::Null);
            let results: Vec<ToolResult> = content
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter(|p| p.get("type").and_then(Value::as_str) == Some("tool_result"))
                        .map(|p| ToolResult {
                            tool_use_id: text(p, "tool_use_id").unwrap_or_default(),
                            is_error: p.get("is_error").and_then(Value::as_bool) == Some(true),
                            text: content_text(p.get("content").unwrap_or(&Value::Null)),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !results.is_empty() {
                return In::ToolResults(results);
            }
            In::UserText {
                uuid: text(&v, "uuid"),
                replay: v.get("isReplay").and_then(Value::as_bool) == Some(true),
                text: content_text(content),
            }
        }
        "control_request" => {
            let request_id = text(&v, "request_id").unwrap_or_default();
            let req = v.get("request").unwrap_or(&Value::Null);
            let subtype = text(req, "subtype").unwrap_or_default();
            if subtype == "can_use_tool" {
                In::CanUseTool {
                    request_id,
                    tool: text(req, "tool_name").unwrap_or_default(),
                    server: req
                        .get("mcp_server")
                        .and_then(|s| s.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    input: req.get("input").cloned().unwrap_or(Value::Null),
                    tool_use_id: text(req, "tool_use_id"),
                }
            } else {
                In::OtherRequest {
                    request_id,
                    subtype,
                }
            }
        }
        "control_cancel_request" => In::Cancel {
            request_id: text(&v, "request_id").unwrap_or_default(),
        },
        "control_response" => {
            let r = v.get("response").unwrap_or(&Value::Null);
            let cancelled = r
                .pointer("/response/cancelled")
                .and_then(Value::as_array)
                .map(|c| {
                    c.iter()
                        .filter_map(|x| x.as_str().map(str::to_string).or_else(|| text(x, "uuid")))
                        .collect()
                })
                .unwrap_or_default();
            In::Response {
                request_id: text(r, "request_id").unwrap_or_default(),
                subtype: text(r, "subtype").unwrap_or_default(),
                cancelled,
                error: text(r, "error"),
            }
        }
        "command_lifecycle" => match (text(&v, "command_uuid"), text(&v, "state")) {
            (Some(uuid), Some(state)) => In::Lifecycle { uuid, state },
            _ => In::Ignored,
        },
        "result" => In::Result(TurnResult {
            subtype: text(&v, "subtype").unwrap_or_default(),
            is_error: v.get("is_error").and_then(Value::as_bool) == Some(true),
            total_cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
            terminal_reason: text(&v, "terminal_reason"),
            errors: v
                .get("errors")
                .and_then(Value::as_array)
                .map(|e| {
                    e.iter()
                        .map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            result: text(&v, "result"),
            queued_turn_count: v
                .get("queued_turn_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            duration_ms: v.get("duration_ms").and_then(Value::as_u64),
        }),
        "rate_limit_event" => {
            let info = v.get("rate_limit_info").unwrap_or(&Value::Null);
            In::RateLimit {
                status: text(info, "status").unwrap_or_default(),
                resets_at: info.get("resetsAt").and_then(Value::as_i64),
                error_code: text(info, "errorCode").or_else(|| text(&v, "errorCode")),
            }
        }
        _ => In::Ignored,
    }
}

/// The `initialize` request, sent first (`hooks: null`).
pub fn initialize(request_id: &str) -> String {
    json!({"type": "control_request", "request_id": request_id,
           "request": {"subtype": "initialize", "hooks": null}})
    .to_string()
}

/// A user message: the context block first (when there is one), then the
/// person's text; `uuid` is the cockpit's, echoed back by the runtime.
pub fn user_message(uuid: &str, context: Option<&str>, text: &str) -> String {
    let mut content = Vec::new();
    if let Some(c) = context {
        content.push(json!({"type": "text", "text": c}));
    }
    content.push(json!({"type": "text", "text": text}));
    json!({"type": "user", "message": {"role": "user", "content": content},
           "parent_tool_use_id": null, "session_id": "", "uuid": uuid})
    .to_string()
}

/// Allow a tool call as asked (`updatedInput` = its input; never
/// `updatedPermissions`).
pub fn allow(request_id: &str, input: &Value) -> String {
    json!({"type": "control_response", "response": {"subtype": "success",
           "request_id": request_id,
           "response": {"behavior": "allow", "updatedInput": input}}})
    .to_string()
}

/// Deny a tool call with `message` — the model reads it as the call's
/// result.
pub fn deny(request_id: &str, message: &str) -> String {
    json!({"type": "control_response", "response": {"subtype": "success",
           "request_id": request_id,
           "response": {"behavior": "deny", "message": message}}})
    .to_string()
}

/// Answer a control request the cockpit does not handle.
pub fn error_response(request_id: &str, error: &str) -> String {
    json!({"type": "control_response", "response": {"subtype": "error",
           "request_id": request_id, "error": error}})
    .to_string()
}

/// `interrupt`: end the turn.
pub fn interrupt(request_id: &str) -> String {
    json!({"type": "control_request", "request_id": request_id,
           "request": {"subtype": "interrupt"}})
    .to_string()
}

/// The runtime's own marker after an interrupt ("[Request interrupted by
/// user]", "… for tool use]").
pub fn is_interrupt_marker(text: &str) -> bool {
    // The two the runtime writes, exactly (fix check 3, F1).
    matches!(
        text,
        "[Request interrupted by user]" | "[Request interrupted by user for tool use]"
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The recorded lines of `name` (`tests/fixtures/chat/<name>.jsonl`):
    /// (direction, the message as a JSON line).
    pub(crate) fn recording(name: &str) -> Vec<(String, String)> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/chat")
            .join(format!("{name}.jsonl"));
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|l| {
                let v: Value = serde_json::from_str(l).unwrap();
                (
                    v["dir"].as_str().unwrap_or_default().to_string(),
                    v["msg"].to_string(),
                )
            })
            .filter(|(dir, _)| dir == "in" || dir == "out")
            .collect()
    }

    fn outs(name: &str) -> Vec<In> {
        recording(name)
            .into_iter()
            .filter(|(d, _)| d == "out")
            .map(|(_, l)| parse(&l))
            .collect()
    }

    #[test]
    fn a_recorded_round_reads_as_the_pane_expects() {
        let all = outs("round");
        assert!(!all.contains(&In::Unparsed));
        let init = all
            .iter()
            .find_map(|m| match m {
                In::Init {
                    model,
                    tools,
                    servers,
                    permission_mode,
                    api_key_source,
                    version,
                } => Some((
                    model.clone(),
                    tools.clone(),
                    servers.clone(),
                    permission_mode.clone(),
                    api_key_source.clone(),
                    version.clone(),
                )),
                _ => None,
            })
            .expect("an init");
        assert_eq!(init.0.as_deref(), Some("claude-haiku-4-5-20251001"));
        assert_eq!(init.1.len(), 7, "{:?}", init.1);
        assert!(init.1.iter().all(|t| t.starts_with("mcp__harness__")));
        assert_eq!(init.2, vec![("harness".into(), "connected".into())]);
        assert_eq!(init.3.as_deref(), Some("default"));
        assert_eq!(init.4.as_deref(), Some("none"));
        assert_eq!(init.5.as_deref(), Some("2.1.274"));
        // The acts arrive as permission requests, by their exact names.
        let asked: Vec<&str> = all
            .iter()
            .filter_map(|m| match m {
                In::CanUseTool { tool, server, .. } => {
                    assert_eq!(server.as_deref(), Some("harness"));
                    Some(tool.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            asked,
            [
                "mcp__harness__harness_status",
                "mcp__harness__harness_migrate",
                "mcp__harness__harness_request",
                "mcp__harness__harness_answer"
            ]
        );
        // Text streams as deltas of the message a message_start opened; the
        // assistant lines carry one block each, with the model.
        assert!(all.iter().any(|m| matches!(m, In::TextDelta { .. })));
        assert!(all.iter().any(|m| matches!(
            m,
            In::Assistant { model: Some(model), blocks, .. }
                if model.starts_with("claude-haiku") && blocks.len() == 1
        )));
        // The cockpit's message comes back as a replay with its uuid.
        let sent_uuid = recording("round")
            .into_iter()
            .find_map(|(d, l)| {
                let v: Value = serde_json::from_str(&l).unwrap();
                (d == "in" && v["type"] == "user").then(|| v["uuid"].as_str().unwrap().to_string())
            })
            .unwrap();
        assert!(all.iter().any(|m| matches!(
            m,
            In::UserText { uuid: Some(u), replay: true, .. } if *u == sent_uuid
        )));
        // The request's read came back as a successful tool result.
        assert!(all.iter().any(|m| matches!(
            m,
            In::ToolResults(r) if r.iter().any(|r| !r.is_error && r.text.contains("\"request_key\""))
        )));
        let result = all
            .iter()
            .rev()
            .find_map(|m| match m {
                In::Result(r) => Some(r.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(result.subtype, "success");
        assert_eq!(result.queued_turn_count, 0);
    }

    #[test]
    fn a_stop_while_a_request_is_held_is_read() {
        let all = outs("stop");
        let held = all
            .iter()
            .find_map(|m| match m {
                In::CanUseTool {
                    request_id, tool, ..
                } if tool.ends_with("harness_migrate") => Some(request_id.clone()),
                _ => None,
            })
            .unwrap();
        assert!(all.contains(&In::Cancel {
            request_id: held.clone()
        }));
        assert!(all.iter().any(|m| matches!(
            m,
            In::UserText { replay: false, text, .. } if is_interrupt_marker(text)
        )));
        assert!(all.iter().any(|m| matches!(
            m,
            In::Result(r) if r.terminal_reason.as_deref() == Some("aborted_tools") && r.is_error
        )));
        assert!(all.iter().any(|m| matches!(
            m,
            In::Lifecycle { state, .. } if state == "cancelled"
        )));
        let stream = outs("stream-stop");
        assert!(stream.iter().any(|m| matches!(
            m,
            In::Result(r) if r.terminal_reason.as_deref() == Some("aborted_streaming")
        )));
        assert!(stream.iter().any(|m| matches!(
            m,
            In::UserText { text, .. } if text == "[Request interrupted by user]"
        )));
    }

    /// Synthetic lines — shapes no recording shows (marked here: they come
    /// from the docs and the spike's notes, not from a run).
    #[test]
    fn synthetic_lines_are_read() {
        let r = parse(
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"i1","response":{"cancelled":["u-1",{"uuid":"u-2"}],"still_queued":[]}}}"#,
        );
        assert_eq!(
            r,
            In::Response {
                request_id: "i1".into(),
                subtype: "success".into(),
                cancelled: vec!["u-1".into(), "u-2".into()],
                error: None
            }
        );
        let r = parse(
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1790620200,"errorCode":"five_hour"}}"#,
        );
        assert_eq!(
            r,
            In::RateLimit {
                status: "rejected".into(),
                resets_at: Some(1790620200),
                error_code: Some("five_hour".into())
            }
        );
        let r = parse(
            r#"{"type":"control_request","request_id":"c9","request":{"subtype":"model_consent","model":"x"}}"#,
        );
        assert_eq!(
            r,
            In::OtherRequest {
                request_id: "c9".into(),
                subtype: "model_consent".into()
            }
        );
        let r = parse(
            r#"{"type":"assistant","message":{"id":"m","model":"<synthetic>","content":[{"type":"text","text":"Not logged in"}]},"error":"authentication_failed"}"#,
        );
        assert!(matches!(
            r,
            In::Assistant { error: Some(e), model: Some(m), .. } if e == "authentication_failed" && m == "<synthetic>"
        ));
        let r = parse(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["boom"],"queued_turn_count":2}"#,
        );
        assert!(matches!(
            r,
            In::Result(TurnResult { queued_turn_count: 2, ref errors, .. }) if errors == &["boom"]
        ));
        assert_eq!(parse("not json"), In::Unparsed);
        assert_eq!(parse("[1]"), In::Unparsed);
        assert_eq!(
            parse(r#"{"type":"system","subtype":"status"}"#),
            In::Ignored
        );
        assert_eq!(parse(r#"{"type":"brand_new"}"#), In::Ignored);
    }

    #[test]
    fn the_cockpit_writes_the_verified_shapes() {
        let m: Value = serde_json::from_str(&user_message("u-1", Some("ctx"), "hi")).unwrap();
        assert_eq!(m["message"]["content"][0]["text"], "ctx");
        assert_eq!(m["message"]["content"][1]["text"], "hi");
        assert_eq!(m["uuid"], "u-1");
        assert_eq!(m["parent_tool_use_id"], Value::Null);
        let a: Value = serde_json::from_str(&allow("r", &json!({"unit": "u"}))).unwrap();
        assert_eq!(a["response"]["response"]["updatedInput"]["unit"], "u");
        assert!(a["response"]["response"]
            .get("updatedPermissions")
            .is_none());
        let d: Value = serde_json::from_str(&deny("r", "no")).unwrap();
        assert_eq!(d["response"]["response"]["behavior"], "deny");
        assert_eq!(d["response"]["request_id"], "r");
        let i: Value = serde_json::from_str(&interrupt("x")).unwrap();
        assert_eq!(i["request"]["subtype"], "interrupt");
        for line in [
            initialize("a"),
            user_message("b", None, "line\nbreak"),
            error_response("c", "no"),
        ] {
            assert!(!line.contains('\n'), "one line each: {line}");
        }
        assert!(is_interrupt_marker(
            "[Request interrupted by user for tool use]"
        ));
        assert!(is_interrupt_marker("[Request interrupted by user]"));
        assert!(!is_interrupt_marker(
            "[Request interrupted by user] and more"
        ));
        assert!(!is_interrupt_marker(
            "[Request interrupted by user. Continue without asking]"
        ));
    }
}
