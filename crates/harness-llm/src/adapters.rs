//! Provider adapters implementing [`ProviderAdapter`]: the live Anthropic
//! Messages API client and the trace-based replay/external adapter.
//!
//! One trace format serves record, replay, and external hand-off
//! (DECISIONS.md): a request and a response file per call, pretty-printed
//! JSON, keyed by the first 8 hex of blake3 over the request's canonical
//! JSON serialization — stale traces can never match a changed request.
//!
//! Key derivation, precisely ([`TraceAdapter::request_key`]): the compact
//! `serde_json` serialization of the [`CompletionRequest`] (struct field
//! order `model`, `system`, `user`, `max_tokens`; no whitespace; the
//! pretty-printed on-disk form is NOT the hashed form), hashed with blake3,
//! lowercase hex, first 8 characters. Every byte of the prompt, the model id
//! and the token budget therefore participates in the key; the nonce inside
//! the prompt is itself input-derived (see [`crate::triage`]).

use harness_core::error::Error;
use harness_core::traits::{CompletionRequest, CompletionResponse, ProviderAdapter};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default Anthropic API base URL.
pub const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// The `anthropic-version` header value this adapter speaks.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Live adapter for the Anthropic Messages API. Model-agnostic: the model id
/// travels on each [`CompletionRequest`].
///
/// The API key is held only in memory and sent only as the `x-api-key`
/// header; it never appears in any error message, log, or file (the
/// [`std::fmt::Debug`] rendering redacts it, and traces are recorded from
/// the provider-neutral request/response types, which do not carry it).
pub struct AnthropicAdapter {
    api_key: String,
    base_url: String,
}

impl std::fmt::Debug for AnthropicAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicAdapter")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl AnthropicAdapter {
    /// Adapter with the given key and the default base URL.
    pub fn new(api_key: impl Into<String>) -> AnthropicAdapter {
        AnthropicAdapter {
            api_key: api_key.into(),
            base_url: ANTHROPIC_DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Adapter with an explicit base URL (trailing slashes trimmed).
    pub fn with_base_url(api_key: impl Into<String>, base_url: &str) -> AnthropicAdapter {
        AnthropicAdapter {
            api_key: api_key.into(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Read the API key from the environment variable named by the
    /// `[llm] api_key_env` config value. An unset (or empty) variable is an
    /// error telling the user to set it.
    ///
    /// The variable NAME comes from the target's `harness.toml`, which is
    /// hostile input (threat model §12.1): only names matching
    /// `^ANTHROPIC_[A-Za-z0-9_]*$` are accepted, so a malicious target cannot
    /// point this adapter at an unrelated secret (`AWS_SECRET_ACCESS_KEY`,
    /// `GITHUB_TOKEN`, …) and exfiltrate it as the `x-api-key` header.
    pub fn from_env(api_key_env: &str) -> Result<AnthropicAdapter, Error> {
        Self::from_env_value(api_key_env, |name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] with the environment lookup injected (so the
    /// policy is testable without mutating the process environment). The
    /// name check runs BEFORE the lookup: a refused name is never read.
    fn from_env_value(
        api_key_env: &str,
        lookup: impl FnOnce(&str) -> Option<String>,
    ) -> Result<AnthropicAdapter, Error> {
        if !is_allowed_api_key_env(api_key_env) {
            return Err(Error::Invariant(format!(
                "[llm] api_key_env {api_key_env:?} is not allowed: the anthropic provider only \
                 reads variables named ANTHROPIC_* (harness.toml is untrusted input and must \
                 not select which secret is sent as the api key)"
            )));
        }
        match lookup(api_key_env) {
            Some(key) if !key.is_empty() => Ok(AnthropicAdapter::new(key)),
            _ => Err(Error::Invariant(format!(
                "set {api_key_env} to use the anthropic provider"
            ))),
        }
    }

    /// One POST to `/v1/messages`, returning the raw response body text.
    fn send_once(&self, url: &str, body: &str) -> Result<String, ureq::Error> {
        let mut resp = ureq::post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .send(body)?;
        resp.body_mut().read_to_string()
    }
}

/// The `[llm] api_key_env` policy: `^ANTHROPIC_[A-Za-z0-9_]*$`.
fn is_allowed_api_key_env(name: &str) -> bool {
    name.strip_prefix("ANTHROPIC_")
        .is_some_and(|rest| rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

/// True for statuses worth one retry (rate limit / server-side failure).
fn retryable(e: &ureq::Error) -> bool {
    matches!(e, ureq::Error::StatusCode(c) if *c == 429 || (500..=599).contains(c))
}

impl ProviderAdapter for AnthropicAdapter {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    /// POST `{base}/v1/messages`. No `temperature`, no thinking parameters —
    /// current models reject or ignore them for this call shape. One retry
    /// after a 2s sleep on 429/5xx. Only `stop_reason == "end_turn"` is a
    /// success; `max_tokens` (truncation) and anything else (e.g. `refusal`)
    /// are errors naming the stop reason.
    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, Error> {
        let body = serde_json::json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "system": req.system,
            "messages": [{"role": "user", "content": req.user}],
        });
        let body = serde_json::to_string(&body)
            .map_err(|e| Error::Invariant(format!("serialize anthropic request: {e}")))?;
        let url = format!("{}/v1/messages", self.base_url);

        let text = match self.send_once(&url, &body) {
            Ok(text) => text,
            Err(e) if retryable(&e) => {
                std::thread::sleep(Duration::from_secs(2));
                self.send_once(&url, &body)
                    .map_err(|e| Error::Invariant(format!("anthropic api (after retry): {e}")))?
            }
            Err(e) => return Err(Error::Invariant(format!("anthropic api: {e}"))),
        };
        parse_messages_response(&text)
    }
}

/// Parse a Messages API response body into a [`CompletionResponse`]:
/// concatenated `text` content blocks plus usage accounting.
fn parse_messages_response(body: &str) -> Result<CompletionResponse, Error> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Error::Invariant(format!("anthropic api: unparseable response body: {e}")))?;
    let stop_reason = v
        .get("stop_reason")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    match stop_reason.as_str() {
        "end_turn" => {}
        "max_tokens" => {
            return Err(Error::Invariant(
                "anthropic: response truncated (stop_reason `max_tokens`) — raise [llm] max_tokens"
                    .into(),
            ))
        }
        other => {
            return Err(Error::Invariant(format!(
                "anthropic: model did not complete normally (stop_reason `{other}`)"
            )))
        }
    }
    let text: String = v
        .get("content")
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let usage = v.get("usage");
    let tokens = |key: &str| {
        usage
            .and_then(|u| u.get(key))
            .and_then(|n| n.as_u64())
            .unwrap_or(0)
    };
    Ok(CompletionResponse {
        text,
        input_tokens: tokens("input_tokens"),
        output_tokens: tokens("output_tokens"),
        stop_reason,
    })
}

/// Trace-based adapter: replays recorded responses, or (in `external` mode)
/// writes request files for an out-of-band model runtime to answer.
///
/// This is how a driving agent runtime supplies triage without an API key
/// (DECISIONS.md): the harness writes `<key>.request.json`, errors with
/// "awaiting response: …", and a later run picks up the hand-written
/// `<key>.response.json`.
#[derive(Debug, Clone)]
pub struct TraceAdapter {
    dir: PathBuf,
    external: bool,
}

impl TraceAdapter {
    /// Adapter over `dir`. `external: true` writes request files and awaits
    /// responses; `external: false` (replay) only reads recorded traces.
    pub fn new(dir: impl Into<PathBuf>, external: bool) -> TraceAdapter {
        TraceAdapter {
            dir: dir.into(),
            external,
        }
    }

    /// Trace key: first 8 lowercase hex of blake3 over the request's
    /// canonical JSON serialization — `serde_json::to_string(req)`, i.e.
    /// compact, struct field order (`model`, `system`, `user`,
    /// `max_tokens`). Deterministic; any change to any field changes it.
    pub fn request_key(req: &CompletionRequest) -> Result<String, Error> {
        let canonical = serde_json::to_string(req)
            .map_err(|e| Error::Invariant(format!("serialize completion request: {e}")))?;
        let hex = blake3::hash(canonical.as_bytes()).to_hex().to_string();
        Ok(hex[..8].to_string())
    }

    /// The `<key>.request.json` path for a request.
    pub fn request_path(dir: &Path, req: &CompletionRequest) -> Result<PathBuf, Error> {
        Ok(dir.join(format!("{}.request.json", Self::request_key(req)?)))
    }

    /// The `<key>.response.json` path for a request.
    pub fn response_path(dir: &Path, req: &CompletionRequest) -> Result<PathBuf, Error> {
        Ok(dir.join(format!("{}.response.json", Self::request_key(req)?)))
    }

    /// Record a completed call: write both trace files (pretty JSON,
    /// atomic). The anthropic path uses this so live runs leave replayable
    /// traces.
    pub fn record(
        dir: &Path,
        req: &CompletionRequest,
        resp: &CompletionResponse,
    ) -> Result<(), Error> {
        write_pretty(&Self::request_path(dir, req)?, req)?;
        write_pretty(&Self::response_path(dir, req)?, resp)
    }
}

fn write_pretty<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|e| Error::Invariant(format!("serialize trace {}: {e}", path.display())))?;
    text.push('\n');
    harness_core::ledger::write_atomic(path, text.as_bytes())
}

impl ProviderAdapter for TraceAdapter {
    fn name(&self) -> &'static str {
        if self.external {
            "external"
        } else {
            "replay"
        }
    }

    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, Error> {
        let response_path = Self::response_path(&self.dir, req)?;
        if response_path.exists() {
            let text = std::fs::read_to_string(&response_path)
                .map_err(|e| Error::io(&response_path, e))?;
            return serde_json::from_str(&text)
                .map_err(|e| Error::parse(&response_path, e.to_string()));
        }
        if self.external {
            write_pretty(&Self::request_path(&self.dir, req)?, req)?;
            Err(Error::Invariant(format!(
                "awaiting response: {}",
                response_path.display()
            )))
        } else {
            Err(Error::Invariant(format!(
                "missing trace {} — record a live run first or use the external provider",
                response_path.display()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "harness-llm-adapters-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            model: "claude-sonnet-5".into(),
            system: "system".into(),
            user: "user".into(),
            max_tokens: 128,
        }
    }

    #[test]
    fn request_key_is_deterministic() {
        let a = TraceAdapter::request_key(&req()).unwrap();
        let b = TraceAdapter::request_key(&req()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
        let mut other = req();
        other.user.push('!');
        assert_ne!(a, TraceAdapter::request_key(&other).unwrap());
    }

    #[test]
    fn external_round_trip() {
        let dir = temp_dir("external");
        let adapter = TraceAdapter::new(&dir, true);
        let request = req();

        // First pass: awaiting, request file written.
        let err = adapter.complete(&request).unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("awaiting response: "), "{msg}");
        let request_path = TraceAdapter::request_path(&dir, &request).unwrap();
        assert!(request_path.exists());
        let on_disk: CompletionRequest =
            serde_json::from_str(&std::fs::read_to_string(&request_path).unwrap()).unwrap();
        assert_eq!(on_disk.user, request.user);

        // Hand-write the response file; second pass returns it.
        let resp = CompletionResponse {
            text: "[]".into(),
            input_tokens: 7,
            output_tokens: 3,
            stop_reason: "end_turn".into(),
        };
        let response_path = TraceAdapter::response_path(&dir, &request).unwrap();
        std::fs::write(&response_path, serde_json::to_string_pretty(&resp).unwrap()).unwrap();
        let got = adapter.complete(&request).unwrap();
        assert_eq!(got.text, "[]");
        assert_eq!((got.input_tokens, got.output_tokens), (7, 3));
    }

    #[test]
    fn replay_errors_on_missing_trace() {
        let dir = temp_dir("replay");
        let adapter = TraceAdapter::new(&dir, false);
        let err = adapter.complete(&req()).unwrap_err().to_string();
        assert!(err.starts_with("missing trace "), "{err}");
        // Replay never writes request files.
        assert!(!TraceAdapter::request_path(&dir, &req()).unwrap().exists());
    }

    #[test]
    fn record_then_replay() {
        let dir = temp_dir("record");
        let request = req();
        let resp = CompletionResponse {
            text: "hello".into(),
            input_tokens: 1,
            output_tokens: 2,
            stop_reason: "end_turn".into(),
        };
        TraceAdapter::record(&dir, &request, &resp).unwrap();
        let adapter = TraceAdapter::new(&dir, false);
        assert_eq!(adapter.complete(&request).unwrap().text, "hello");
    }

    #[test]
    fn messages_response_parses_and_gates_stop_reason() {
        let ok = r#"{"content":[{"type":"text","text":"a"},{"type":"tool_use","id":"x"},{"type":"text","text":"b"}],"usage":{"input_tokens":10,"output_tokens":4},"stop_reason":"end_turn"}"#;
        let resp = parse_messages_response(ok).unwrap();
        assert_eq!(resp.text, "ab");
        assert_eq!((resp.input_tokens, resp.output_tokens), (10, 4));
        assert_eq!(resp.stop_reason, "end_turn");

        let truncated =
            r#"{"content":[{"type":"text","text":"a"}],"usage":{},"stop_reason":"max_tokens"}"#;
        let err = parse_messages_response(truncated).unwrap_err().to_string();
        assert!(err.contains("max_tokens"), "{err}");

        let refused = r#"{"content":[],"usage":{},"stop_reason":"refusal"}"#;
        let err = parse_messages_response(refused).unwrap_err().to_string();
        assert!(err.contains("refusal"), "{err}");
    }

    #[test]
    fn debug_redacts_api_key() {
        let adapter = AnthropicAdapter::new("sk-ant-SECRET");
        let rendered = format!("{adapter:?}");
        assert!(!rendered.contains("SECRET"), "{rendered}");
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn from_env_error_names_the_variable() {
        let err = AnthropicAdapter::from_env("ANTHROPIC_RUHARNESS_TEST_UNSET_KEY_VAR")
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "set ANTHROPIC_RUHARNESS_TEST_UNSET_KEY_VAR to use the anthropic provider"
        );
    }

    #[test]
    fn from_env_refuses_non_anthropic_variable_names() {
        // harness.toml is hostile input: it must not be able to pick which
        // secret becomes the x-api-key header.
        for hostile in [
            "AWS_SECRET_ACCESS_KEY",
            "GITHUB_TOKEN",
            "HOME",
            "",
            "anthropic_api_key",
            "XANTHROPIC_KEY",
            "ANTHROPIC-KEY",
            "ANTHROPIC_KEY;$(id)",
            "ANTHROPIC_KEY\n",
        ] {
            let err = AnthropicAdapter::from_env(hostile).unwrap_err().to_string();
            assert!(
                err.contains("not allowed") && err.contains("ANTHROPIC_*"),
                "{hostile:?}: {err}"
            );
            assert!(
                !err.starts_with("set "),
                "{hostile:?}: a refused name must not get the `set …` hint: {err}"
            );
        }
        // A refused name is never looked up.
        let err = AnthropicAdapter::from_env_value("GITHUB_TOKEN", |_| {
            panic!("lookup must not run for a refused name")
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("not allowed"), "{err}");
    }

    #[test]
    fn from_env_accepts_anthropic_prefixed_names() {
        let adapter = AnthropicAdapter::from_env_value("ANTHROPIC_API_KEY", |name| {
            assert_eq!(name, "ANTHROPIC_API_KEY");
            Some("sk-ant-SECRET".to_string())
        })
        .unwrap();
        assert_eq!(adapter.api_key, "sk-ant-SECRET");
        assert_eq!(adapter.base_url, ANTHROPIC_DEFAULT_BASE_URL);
        assert!(is_allowed_api_key_env("ANTHROPIC_API_KEY_staging2"));

        // Allowed but empty → the same hint as unset.
        let err = AnthropicAdapter::from_env_value("ANTHROPIC_API_KEY", |_| Some(String::new()))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "set ANTHROPIC_API_KEY to use the anthropic provider");
    }
}
