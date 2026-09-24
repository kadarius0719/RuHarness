//! JSON-RPC 2.0 over MCP's stdio transport (docs/MCP-DESIGN.md §2):
//! newline-delimited UTF-8 messages, one per line, a line over
//! [`MAX_LINE_BYTES`] refused. Batches (removed from MCP in 2025-06-18) are
//! invalid requests.

use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// Longest line read, in bytes (without its `\n`).
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// JSON-RPC error codes.
pub const PARSE_ERROR: i64 = -32700;
/// The message is not a valid request.
pub const INVALID_REQUEST: i64 = -32600;
/// No such method.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Bad parameters (MCP: also an unknown tool and arguments that do not
/// match its schema).
pub const INVALID_PARAMS: i64 = -32602;

/// One line of input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// The line's bytes, without `\n` (and a trailing `\r`).
    Line(Vec<u8>),
    /// A line longer than the cap; its bytes were read and discarded.
    TooLong,
}

/// Read one line of at most `cap` bytes; the bytes of a longer line are
/// consumed and dropped (never buffered), so no line can exhaust memory.
/// `None` at EOF.
pub fn read_frame(reader: &mut impl BufRead, cap: usize) -> std::io::Result<Option<Frame>> {
    let mut line = Vec::new();
    let mut too_long = false;
    let mut seen_any = false;
    loop {
        let (consumed, done) = {
            let buf = match reader.fill_buf() {
                Ok(buf) => buf,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if buf.is_empty() {
                if !seen_any {
                    return Ok(None);
                }
                break;
            }
            seen_any = true;
            let (chunk, consumed, done) = match buf.iter().position(|b| *b == b'\n') {
                Some(i) => (&buf[..i], i + 1, true),
                None => (buf, buf.len(), false),
            };
            if line.len() + chunk.len() > cap {
                too_long = true;
                line.clear();
            } else if !too_long {
                line.extend_from_slice(chunk);
            }
            (consumed, done)
        };
        reader.consume(consumed);
        if done {
            break;
        }
    }
    if too_long {
        return Ok(Some(Frame::TooLong));
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Some(Frame::Line(line)))
}

/// What a line is.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A request: it gets exactly one response.
    Request {
        /// Its id (a string or an integer).
        id: Value,
        /// The method.
        method: String,
        /// Its params (an object or an array), when present.
        params: Option<Value>,
    },
    /// A notification: never answered.
    Notification {
        /// The method.
        method: String,
        /// Its params, when present.
        params: Option<Value>,
    },
    /// A response to a request of ours (this server sends none): ignored.
    Response,
    /// Not a valid message: answered with this error and `id: null`.
    Invalid {
        /// `PARSE_ERROR` or `INVALID_REQUEST`.
        code: i64,
        /// Why.
        message: &'static str,
    },
    /// A blank line: ignored.
    Blank,
}

fn valid_id(id: &Value) -> bool {
    id.is_string() || id.is_i64() || id.is_u64()
}

/// Classify one line.
pub fn classify(line: &[u8]) -> Incoming {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Incoming::Blank;
    }
    let invalid = |message| Incoming::Invalid {
        code: INVALID_REQUEST,
        message,
    };
    let Ok(value) = serde_json::from_slice::<Value>(line) else {
        return Incoming::Invalid {
            code: PARSE_ERROR,
            message: "parse error: not a JSON value in UTF-8",
        };
    };
    let Some(obj) = value.as_object() else {
        return if value.is_array() {
            invalid("batches are not supported (MCP 2025-06-18)")
        } else {
            invalid("not a JSON-RPC message object")
        };
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return invalid("jsonrpc must be \"2.0\"");
    }
    let params = obj.get("params").cloned();
    if params
        .as_ref()
        .is_some_and(|p| !p.is_object() && !p.is_array())
    {
        return invalid("params must be an object or an array");
    }
    match (obj.get("method"), obj.get("id")) {
        (Some(Value::String(method)), None) => Incoming::Notification {
            method: method.clone(),
            params,
        },
        (Some(Value::String(method)), Some(id)) if valid_id(id) => Incoming::Request {
            id: id.clone(),
            method: method.clone(),
            params,
        },
        (Some(Value::String(_)), Some(_)) => invalid("id must be a string or an integer"),
        (Some(_), _) => invalid("method must be a string"),
        (None, Some(id))
            if valid_id(id) && (obj.contains_key("result") || obj.contains_key("error")) =>
        {
            Incoming::Response
        }
        (None, _) => invalid("not a request, a notification or a response"),
    }
}

/// A success response.
pub fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// An error response.
pub fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// A notification.
pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

/// Write one message as one line and flush. Compact serde_json never
/// emits a raw newline (strings escape it), so a message is one line.
pub fn send(out: &mut impl Write, message: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_string(message).map_err(std::io::Error::other)?;
    line.push('\n');
    out.write_all(line.as_bytes())?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(input: &[u8], cap: usize) -> Vec<Frame> {
        let mut reader = std::io::BufReader::with_capacity(4, input);
        let mut out = Vec::new();
        while let Some(f) = read_frame(&mut reader, cap).unwrap() {
            out.push(f);
        }
        out
    }

    #[test]
    fn lines_are_split_capped_and_never_buffered_past_the_cap() {
        assert_eq!(
            frames(b"ab\r\ncdefgh\nij", 4),
            vec![
                Frame::Line(b"ab".to_vec()),
                Frame::TooLong,
                Frame::Line(b"ij".to_vec())
            ]
        );
        // Exactly the cap is fine; one more byte is not.
        assert_eq!(frames(b"abcd\n", 4), vec![Frame::Line(b"abcd".to_vec())]);
        assert_eq!(frames(b"abcde\n", 4), vec![Frame::TooLong]);
        assert!(frames(b"", 4).is_empty());
    }

    #[test]
    fn messages_are_classified() {
        let c = |s: &str| classify(s.as_bytes());
        assert!(matches!(
            c(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#),
            Incoming::Request { method, .. } if method == "ping"
        ));
        assert!(matches!(
            c(r#"{"jsonrpc":"2.0","id":"x","method":"ping","params":{}}"#),
            Incoming::Request { .. }
        ));
        assert!(matches!(
            c(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Incoming::Notification { .. }
        ));
        assert_eq!(
            c(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#),
            Incoming::Response
        );
        assert_eq!(c("  "), Incoming::Blank);
        let code = |s: &str| match c(s) {
            Incoming::Invalid { code, .. } => code,
            other => panic!("{other:?}"),
        };
        assert_eq!(code("{nope"), PARSE_ERROR);
        assert_eq!(code("[1]"), INVALID_REQUEST);
        assert_eq!(
            code(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#),
            INVALID_REQUEST
        );
        assert_eq!(code(r#"{"id":1,"method":"ping"}"#), INVALID_REQUEST);
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#),
            INVALID_REQUEST
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#),
            INVALID_REQUEST
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":1,"method":7}"#),
            INVALID_REQUEST
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":1,"method":"x","params":3}"#),
            INVALID_REQUEST
        );
        assert_eq!(code(r#"{"jsonrpc":"2.0"}"#), INVALID_REQUEST);
        assert_eq!(code("\"text\""), INVALID_REQUEST);
        // Invalid UTF-8 is a parse error, never replaced.
        assert_eq!(
            match classify(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"\xff\"}") {
                Incoming::Invalid { code, .. } => code,
                other => panic!("{other:?}"),
            },
            PARSE_ERROR
        );
    }

    #[test]
    fn a_message_is_one_line() {
        let mut out = Vec::new();
        send(&mut out, &result(&json!(1), json!({"text": "a\nb"}))).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.matches('\n').count(), 1);
        assert!(text.ends_with('\n'));
    }
}
