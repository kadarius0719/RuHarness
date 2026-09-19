//! The `openai-compat` provider kind: the OpenAI Chat Completions wire
//! format (`POST {base_url}/chat/completions`), as spoken by OpenAI and by
//! the many servers that mirror it (Ollama, llama.cpp, vLLM, LM Studio,
//! OpenRouter, …). It is reached only through a user-level provider profile
//! (docs/SCHEMAS.md "Provider profiles", [`crate::providers`]); there is no
//! built-in profile of this kind.
//!
//! This file plus ONE arm in the providers `kind_builder` match is the whole
//! integration: nothing else in the harness knows this wire format exists.
//! Everything downstream works on the provider-neutral
//! [`CompletionRequest`] / [`CompletionResponse`] pair.
//!
//! ```toml
//! [providers.ollama-openai]
//! kind = "openai-compat"
//! base_url = "http://127.0.0.1:11434/v1"   # INCLUDES any version prefix
//! # api_key_env = "OPENAI_API_KEY"         # optional; omitted = no auth header
//! # max_tokens_field = "max_completion_tokens"   # default "max_tokens"
//! ```
//!
//! Same posture as the Anthropic adapter: explicit timeout (ureq 3 has
//! none), redirects never followed, no sampling parameters, the api key held
//! only in memory and scrubbed from every error, `stop_reason` returned RAW.

use crate::adapters::{is_env_var_name, is_loopback_url, DEFAULT_TIMEOUT_SECS};
use crate::providers::{EnvLookup, ProviderProfile};
use harness_core::error::Error;
use harness_core::traits::{CompletionRequest, CompletionResponse, ProviderAdapter};
use serde_json::Value;
use std::time::Duration;

/// Prefix of every error this adapter produces.
const API: &str = "openai-compat api";

/// Closed value set of the profile's `max_tokens_field` (docs/SCHEMAS.md);
/// the first entry is the default. Only these names can ever become a
/// request-body key.
const MAX_TOKENS_FIELDS: [&str; 2] = ["max_tokens", "max_completion_tokens"];

/// Longest prefix of provider-supplied error text echoed in an error, in
/// chars.
const ERROR_TEXT_MAX_CHARS: usize = 500;

/// Pause before the single retry on 429/5xx.
const RETRY_DELAY: Duration = Duration::from_secs(2);

/// The `stop_reason` reported when the provider's `message.refusal` is set
/// (maps to `StopKind::Refusal`).
const REFUSAL: &str = "refusal";

/// Live adapter for servers speaking the OpenAI Chat Completions format.
/// Model-agnostic: the model id travels on each [`CompletionRequest`], and
/// only ever inside the request body.
///
/// The API key is optional (a local server needs none); when present it is
/// held only in memory and sent only as the `Authorization: Bearer` header.
/// It never appears in any error message, log, or file: the
/// [`std::fmt::Debug`] rendering redacts it, every error string built from
/// provider output is scrubbed of it, and traces are recorded from the
/// provider-neutral request/response types, which do not carry it.
pub(crate) struct OpenAiCompatAdapter {
    api_key: Option<String>,
    base_url: String,
    max_tokens_field: &'static str,
    timeout: Duration,
    retry_delay: Duration,
    agent: ureq::Agent,
}

impl std::fmt::Debug for OpenAiCompatAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatAdapter")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("base_url", &self.base_url)
            .field("max_tokens_field", &self.max_tokens_field)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// [`crate::providers::KindBuilder`] for `openai-compat`.
pub(crate) fn build(
    profile: &ProviderProfile,
    env: EnvLookup<'_>,
) -> Result<Box<dyn ProviderAdapter>, Error> {
    Ok(Box::new(from_profile(profile, env)?))
}

/// The adapter for a validated user-level profile. The profile was already
/// validated by [`crate::providers`]; what decides whether a secret is read
/// or what reaches the wire is re-checked here regardless, so the adapter is
/// safe on its own:
///
/// - `timeout_secs`: `None` = [`DEFAULT_TIMEOUT_SECS`]; `0` is an error.
/// - `max_tokens_field`: `None` = `max_tokens`; anything outside
///   [`MAX_TOKENS_FIELDS`] is an error.
/// - `api_key_env`: `None` = no auth header, and the environment is never
///   read. A value that is not shaped like a variable name is refused
///   unread and NOT echoed (the usual way to get one is pasting the key
///   itself into this field). A named variable that is unset, empty, or not
///   UTF-8 is an error telling the user to set it.
fn from_profile(
    profile: &ProviderProfile,
    env: EnvLookup<'_>,
) -> Result<OpenAiCompatAdapter, Error> {
    let name = &profile.name;
    let timeout_secs = profile.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    if timeout_secs == 0 {
        return Err(Error::Invariant(format!(
            "provider profile `{name}`: timeout_secs must be greater than 0"
        )));
    }
    let max_tokens_field = match profile.max_tokens_field.as_deref() {
        None => MAX_TOKENS_FIELDS[0],
        Some(requested) => MAX_TOKENS_FIELDS
            .into_iter()
            .find(|legal| *legal == requested)
            .ok_or_else(|| {
                Error::Invariant(format!(
                    "provider profile `{name}`: max_tokens_field is not one of: {}",
                    MAX_TOKENS_FIELDS.join(", ")
                ))
            })?,
    };
    let api_key = match profile.api_key_env.as_deref() {
        None => None,
        Some(variable) if !is_env_var_name(variable) => {
            // Deliberately not echoed: the classic mistake is pasting the
            // key itself here, and errors end up in logs.
            return Err(Error::Invariant(format!(
                "provider profile `{name}`: api_key_env is not an environment variable name \
                 (expected ^[A-Za-z_][A-Za-z0-9_]*$; value not shown) — it names the variable \
                 that holds the key, never the key itself"
            )));
        }
        Some(variable) => match env(variable).and_then(|value| value.into_string().ok()) {
            Some(key) if !key.is_empty() => Some(key),
            _ => {
                return Err(Error::Invariant(format!(
                    "set {variable} to use the {name} provider"
                )))
            }
        },
    };

    // Explicit global timeout (ureq 3 has none by default), HTTP error
    // statuses returned as responses (so error bodies can be read), and
    // redirects disabled (a credentialed request never follows a server's
    // pointer to another host). A loopback endpoint bypasses any `*_PROXY`
    // environment proxy: a proxy cannot reach the user's own loopback, and
    // a local server's traffic should never leave the machine.
    let timeout = Duration::from_secs(timeout_secs);
    let mut config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .max_redirects(0);
    if is_loopback_url(&profile.base_url) {
        config = config.proxy(None);
    }
    Ok(OpenAiCompatAdapter {
        api_key,
        base_url: profile.base_url.trim_end_matches('/').to_string(),
        max_tokens_field,
        timeout,
        retry_delay: RETRY_DELAY,
        agent: config.build().into(),
    })
}

/// True for statuses worth one retry (rate limit / server-side failure).
fn retryable(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

/// The human-readable part of a provider `error` value: a bare string, or
/// an object's `message` (plus its `code`, when present); anything else is
/// rendered as compact JSON.
fn error_detail(error: &Value) -> String {
    if let Value::String(message) = error {
        return message.clone();
    }
    match error.get("message").and_then(Value::as_str) {
        Some(message) if !message.is_empty() => match error.get("code") {
            Some(code) if !code.is_null() => format!("{message} (code {code})"),
            _ => message.to_string(),
        },
        _ => error.to_string(),
    }
}

impl OpenAiCompatAdapter {
    /// One POST, returning the HTTP status and the raw response body text.
    /// `Authorization` is sent only when a key is configured. For an error
    /// status an unreadable body degrades to an empty one rather than
    /// masking the status.
    fn send_once(&self, url: &str, body: &str) -> Result<(u16, String), ureq::Error> {
        let mut request = self
            .agent
            .post(url)
            .header("content-type", "application/json");
        if let Some(key) = &self.api_key {
            request = request.header("authorization", format!("Bearer {key}"));
        }
        let mut resp = request.send(body)?;
        let status = resp.status().as_u16();
        let text = if (200..300).contains(&status) {
            resp.body_mut().read_to_string()?
        } else {
            resp.body_mut().read_to_string().unwrap_or_default()
        };
        Ok((status, text))
    }

    /// Replace every occurrence of the api key in `text` — provider output
    /// and transport errors are untrusted and may echo request headers.
    fn scrub(&self, text: &str) -> String {
        match &self.api_key {
            Some(key) => text.replace(key.as_str(), "<redacted>"),
            None => text.to_string(),
        }
    }

    /// Untrusted provider text made fit for an error message: api key
    /// scrubbed FIRST, control characters flattened to spaces (the message
    /// stays single-line), trimmed, then capped at [`ERROR_TEXT_MAX_CHARS`]
    /// chars.
    fn sanitize(&self, text: &str) -> String {
        let flattened: String = self
            .scrub(text)
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let flattened = flattened.trim();
        let mut shown: String = flattened.chars().take(ERROR_TEXT_MAX_CHARS).collect();
        if shown.len() < flattened.len() {
            shown.push('…');
        }
        shown
    }

    /// Error for a transport-level failure (no HTTP status was received).
    /// `phase` is `""` or `" (after retry)"`.
    fn transport_error(&self, phase: &str, e: &ureq::Error) -> Error {
        let detail = match e {
            ureq::Error::Timeout(which) => format!(
                "timed out after {}s ({which}) — raise the provider profile's timeout_secs",
                self.timeout.as_secs()
            ),
            other => other.to_string(),
        };
        Error::Invariant(self.scrub(&format!("{API}{phase}: {detail}")))
    }

    /// Error for a non-success HTTP status, surfacing the provider's
    /// (sanitized) error body.
    fn status_error(&self, phase: &str, status: u16, body: &str) -> Error {
        let shown = self.sanitize(body);
        Error::Invariant(if shown.is_empty() {
            format!("{API}{phase}: http status {status}")
        } else {
            format!("{API}{phase}: http status {status}: {shown}")
        })
    }

    /// Error for a failure reported INSIDE an HTTP 200 body.
    fn body_error(&self, error: &Value) -> Error {
        Error::Invariant(format!(
            "{API}: error in response body: {}",
            self.sanitize(&error_detail(error))
        ))
    }

    /// Parse a Chat Completions response body into a [`CompletionResponse`].
    ///
    /// An HTTP 200 can still carry a failure (OpenRouter style): a top-level
    /// `error`, a `choices[0].error`, or `choices[0].finish_reason ==
    /// "error"` is an `Err`, as is a body without a first choice. Otherwise:
    ///
    /// - `text` = `choices[0].message.content` (null/absent → `""`; the
    ///   array-of-parts form contributes its `text` parts). `<think>` blocks
    ///   are NOT stripped here — the emission parser owns that.
    /// - `stop_reason` = `"refusal"` when `choices[0].message.refusal` is a
    ///   non-empty string, else the RAW `finish_reason` (absent/null →
    ///   `""`). `length`, `content_filter`, … are never errors at this
    ///   layer: callers decide via [`CompletionResponse::stop`].
    /// - `usage.prompt_tokens` / `usage.completion_tokens` → input / output
    ///   tokens; missing → `0`, which callers treat as "not reported".
    fn parse_chat_response(&self, body: &str) -> Result<CompletionResponse, Error> {
        let v: Value = serde_json::from_str(body)
            .map_err(|e| Error::Invariant(format!("{API}: unparseable response body: {e}")))?;
        let reported = |holder: &Value| holder.get("error").filter(|e| !e.is_null()).cloned();
        if let Some(error) = reported(&v) {
            return Err(self.body_error(&error));
        }
        let Some(choice) = v
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .filter(|choice| choice.is_object())
        else {
            return Err(Error::Invariant(format!(
                "{API}: response has no choices: {}",
                self.sanitize(body)
            )));
        };
        if let Some(error) = reported(choice) {
            return Err(self.body_error(&error));
        }
        let finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or("");
        if finish_reason == "error" {
            return Err(Error::Invariant(format!(
                "{API}: error in response body: finish_reason \"error\" with no error detail"
            )));
        }

        let message = choice.get("message");
        let text: String = match message.and_then(|m| m.get("content")) {
            Some(Value::String(content)) => content.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect(),
            _ => String::new(),
        };
        let refused = message
            .and_then(|m| m.get("refusal"))
            .and_then(Value::as_str)
            .is_some_and(|refusal| !refusal.is_empty());
        let usage = v.get("usage");
        let tokens = |key: &str| {
            usage
                .and_then(|u| u.get(key))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        Ok(CompletionResponse {
            text,
            input_tokens: tokens("prompt_tokens"),
            output_tokens: tokens("completion_tokens"),
            stop_reason: if refused { REFUSAL } else { finish_reason }.to_string(),
        })
    }
}

impl ProviderAdapter for OpenAiCompatAdapter {
    fn name(&self) -> &'static str {
        "openai-compat"
    }

    /// POST `{base_url}/chat/completions` — `base_url` already carries any
    /// version prefix (`…/v1`), none is appended — with exactly `model`,
    /// the profile's max-tokens field, and `messages` (system, then user).
    /// Adapters never send sampling parameters (docs/SCHEMAS.md), nor
    /// `stream`. One retry after a pause on 429/5xx; any other non-2xx
    /// status is an error carrying the (scrubbed, truncated) provider error
    /// body.
    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, Error> {
        let mut body = serde_json::Map::new();
        body.insert("model".to_string(), Value::from(req.model.as_str()));
        body.insert(
            self.max_tokens_field.to_string(),
            Value::from(req.max_tokens),
        );
        body.insert(
            "messages".to_string(),
            serde_json::json!([
                {"role": "system", "content": req.system},
                {"role": "user", "content": req.user},
            ]),
        );
        let body = serde_json::to_string(&Value::Object(body))
            .map_err(|e| Error::Invariant(format!("serialize openai-compat request: {e}")))?;
        let url = format!("{}/chat/completions", self.base_url);

        let (mut status, mut text) = self
            .send_once(&url, &body)
            .map_err(|e| self.transport_error("", &e))?;
        let mut phase = "";
        if retryable(status) {
            std::thread::sleep(self.retry_delay);
            phase = " (after retry)";
            (status, text) = self
                .send_once(&url, &body)
                .map_err(|e| self.transport_error(phase, &e))?;
        }
        if !(200..300).contains(&status) {
            return Err(self.status_error(phase, status, &text));
        }
        self.parse_chat_response(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The crate's shared one-shot mock server ("Shared by every adapter's
    // tests in this crate") — reused as-is, adapters.rs is not touched.
    use crate::adapters::mock_http::{response, response_with_headers, serve, serve_once, Reply};
    use crate::adapters::test_env::EnvGuard;
    use harness_core::traits::StopKind;
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Instant;

    const OK_BODY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#;

    /// The variable the test profiles name; it exists only in the injected
    /// environments below, never in the process environment.
    const KEY_ENV: &str = "OPENAI_COMPAT_TEST_KEY";

    fn req() -> CompletionRequest {
        CompletionRequest {
            model: "qwen2.5-coder:7b".into(),
            system: "system".into(),
            user: "user".into(),
            max_tokens: 128,
        }
    }

    fn profile(base_url: &str) -> ProviderProfile {
        ProviderProfile {
            name: "local-openai".into(),
            kind: "openai-compat".into(),
            base_url: base_url.into(),
            api_key_env: None,
            context_tokens: None,
            timeout_secs: Some(30),
            max_tokens_field: None,
        }
    }

    /// An environment that must never be consulted.
    fn no_env(name: &str) -> Option<OsString> {
        panic!("environment must not be read without api_key_env (asked for {name})")
    }

    /// An adapter aimed at `base_url`, with a near-zero retry pause; `key`
    /// = the value of the profile's `api_key_env` variable (`None` = the
    /// profile names no variable).
    fn adapter_for(base_url: &str, key: Option<&str>) -> OpenAiCompatAdapter {
        let mut profile = profile(base_url);
        profile.api_key_env = key.map(|_| KEY_ENV.to_string());
        let mut adapter = match key {
            Some(key) => from_profile(&profile, &|name| {
                assert_eq!(name, KEY_ENV);
                Some(OsString::from(key))
            }),
            None => from_profile(&profile, &no_env),
        }
        .unwrap();
        adapter.retry_delay = Duration::from_millis(10);
        adapter
    }

    /// An adapter for parse-only tests (never connects).
    fn offline(key: Option<&str>) -> OpenAiCompatAdapter {
        adapter_for("http://127.0.0.1:1/v1", key)
    }

    fn body_keys(body: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    // ---- request wire shape ----------------------------------------------

    #[test]
    fn posts_to_chat_completions_under_the_versioned_base_url() {
        // base_url already ends in /v1: nothing is appended but the route,
        // and trailing slashes never produce `//chat/completions`.
        for suffix in ["/v1", "/v1/", "/v1///"] {
            let server = serve_once(response(200, "OK", OK_BODY));
            let adapter = adapter_for(&format!("{}{suffix}", server.base_url), None);
            assert_eq!(adapter.complete(&req()).unwrap().text, "hello");
            let seen = server.finish();
            assert_eq!(seen.len(), 1);
            assert_eq!(
                seen[0].request_line, "POST /v1/chat/completions HTTP/1.1",
                "suffix {suffix:?}"
            );
            assert_eq!(seen[0].header("content-type"), Some("application/json"));
        }

        // No version prefix in the profile → none on the wire.
        let server = serve_once(response(200, "OK", OK_BODY));
        let adapter = adapter_for(&format!("{}/api/openai", server.base_url), None);
        adapter.complete(&req()).unwrap();
        assert_eq!(
            server.finish()[0].request_line,
            "POST /api/openai/chat/completions HTTP/1.1"
        );
    }

    #[test]
    fn authorization_header_only_with_a_configured_key() {
        let server = serve_once(response(200, "OK", OK_BODY));
        let adapter = adapter_for(&format!("{}/v1", server.base_url), Some("sk-test-SECRET"));
        adapter.complete(&req()).unwrap();
        let seen = server.finish();
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer sk-test-SECRET")
        );
        assert_eq!(seen[0].header("x-api-key"), None);
        assert_eq!(seen[0].header("anthropic-version"), None);
        // The key travels only in its header.
        assert!(!seen[0].body.contains("SECRET"));
        assert!(!seen[0].request_line.contains("SECRET"));

        // No api_key_env: no auth header of any kind, environment unread
        // (`adapter_for(_, None)` panics on any lookup).
        let server = serve_once(response(200, "OK", OK_BODY));
        let adapter = adapter_for(&format!("{}/v1", server.base_url), None);
        adapter.complete(&req()).unwrap();
        let seen = server.finish();
        assert_eq!(seen[0].header("authorization"), None);
        assert_eq!(seen[0].header("x-api-key"), None);
    }

    #[test]
    fn body_is_exactly_model_max_tokens_and_messages() {
        let server = serve_once(response(200, "OK", OK_BODY));
        let adapter = adapter_for(&format!("{}/v1", server.base_url), Some("sk-test-SECRET"));
        adapter.complete(&req()).unwrap();
        let seen = server.finish();
        let body: Value = serde_json::from_str(&seen[0].body).unwrap();
        // No temperature, top_p, stream, … — nothing but these three.
        assert_eq!(body_keys(&body), ["max_tokens", "messages", "model"]);
        assert!(body.get("temperature").is_none());
        assert_eq!(
            body,
            serde_json::json!({
                "model": "qwen2.5-coder:7b",
                "max_tokens": 128,
                "messages": [
                    {"role": "system", "content": "system"},
                    {"role": "user", "content": "user"},
                ],
            })
        );
    }

    #[test]
    fn max_completion_tokens_variant_renames_only_the_budget_field() {
        let server = serve_once(response(200, "OK", OK_BODY));
        let mut profile = profile(&format!("{}/v1", server.base_url));
        profile.max_tokens_field = Some("max_completion_tokens".into());
        let adapter = from_profile(&profile, &no_env).unwrap();
        adapter.complete(&req()).unwrap();
        let seen = server.finish();
        let body: Value = serde_json::from_str(&seen[0].body).unwrap();
        assert_eq!(
            body_keys(&body),
            ["max_completion_tokens", "messages", "model"]
        );
        assert_eq!(body["max_completion_tokens"], 128);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("temperature").is_none());

        // The explicit default spelling is accepted too.
        profile.max_tokens_field = Some("max_tokens".into());
        assert_eq!(
            from_profile(&profile, &no_env).unwrap().max_tokens_field,
            "max_tokens"
        );
    }

    // ---- response handling -----------------------------------------------

    #[test]
    fn response_parsing_over_http_maps_usage_and_passes_finish_reason_through() {
        let body = r#"{"id":"chatcmpl-9","model":"qwen2.5-coder:7b","choices":[{"index":0,"message":{"role":"assistant","content":"<think>hm</think>fn main() {"},"finish_reason":"length"},{"index":1,"message":{"role":"assistant","content":"ignored"},"finish_reason":"stop"}],"usage":{"prompt_tokens":9120,"completion_tokens":1400,"total_tokens":10520}}"#;
        let server = serve_once(response(200, "OK", body));
        let adapter = adapter_for(&format!("{}/v1", server.base_url), None);
        // `length` is a successful completion at this layer …
        let resp = adapter.complete(&req()).unwrap();
        server.finish();
        // … only the first choice counts, and <think> is left in place.
        assert_eq!(resp.text, "<think>hm</think>fn main() {");
        assert_eq!((resp.input_tokens, resp.output_tokens), (9120, 1400));
        // The RAW provider string is preserved; the kind is derived.
        assert_eq!(resp.stop_reason, "length");
        assert_eq!(resp.stop(), StopKind::MaxTokens);
    }

    #[test]
    fn finish_reasons_pass_through_raw() {
        let adapter = offline(None);
        for (raw, kind) in [
            ("stop", StopKind::EndTurn),
            ("length", StopKind::MaxTokens),
            ("content_filter", StopKind::Refusal),
            ("tool_calls", StopKind::Other),
        ] {
            let body = format!(
                r#"{{"choices":[{{"message":{{"content":"x"}},"finish_reason":"{raw}"}}]}}"#
            );
            let resp = adapter.parse_chat_response(&body).unwrap();
            assert_eq!(resp.stop_reason, raw);
            assert_eq!(resp.stop(), kind, "{raw}");
        }
        // Absent and null finish reasons are "", never an error.
        for body in [
            r#"{"choices":[{"message":{"content":"x"}}]}"#,
            r#"{"choices":[{"message":{"content":"x"},"finish_reason":null}]}"#,
        ] {
            let resp = adapter.parse_chat_response(body).unwrap();
            assert_eq!((resp.text.as_str(), resp.stop_reason.as_str()), ("x", ""));
            assert_eq!(resp.stop(), StopKind::Other);
        }
    }

    #[test]
    fn content_may_be_null_absent_or_parts() {
        let adapter = offline(None);
        for body in [
            r#"{"choices":[{"message":{"role":"assistant","content":null},"finish_reason":"stop"}]}"#,
            r#"{"choices":[{"message":{"role":"assistant"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[{"finish_reason":"stop"}]}"#,
        ] {
            let resp = adapter.parse_chat_response(body).unwrap();
            assert_eq!(
                (resp.text.as_str(), resp.stop_reason.as_str()),
                ("", "stop")
            );
        }
        let parts = r#"{"choices":[{"message":{"content":[{"type":"text","text":"a"},{"type":"thinking","thinking":"hidden"},{"type":"text","text":"b"}]},"finish_reason":"stop"}]}"#;
        assert_eq!(adapter.parse_chat_response(parts).unwrap().text, "ab");
    }

    #[test]
    fn missing_usage_is_zero_zero() {
        let adapter = offline(None);
        for body in [
            r#"{"choices":[{"message":{"content":"x"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[{"message":{"content":"x"},"finish_reason":"stop"}],"usage":null}"#,
            r#"{"choices":[{"message":{"content":"x"},"finish_reason":"stop"}],"usage":{}}"#,
        ] {
            let resp = adapter.parse_chat_response(body).unwrap();
            assert_eq!((resp.input_tokens, resp.output_tokens), (0, 0), "{body}");
        }
        // One side reported: the other stays 0.
        let half = r#"{"choices":[{"message":{"content":"x"}}],"usage":{"prompt_tokens":7}}"#;
        let resp = adapter.parse_chat_response(half).unwrap();
        assert_eq!((resp.input_tokens, resp.output_tokens), (7, 0));
    }

    #[test]
    fn refusal_field_becomes_the_refusal_stop_reason() {
        let body = r#"{"choices":[{"index":0,"message":{"role":"assistant","content":null,"refusal":"I can't help with that."},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":8}}"#;
        let server = serve_once(response(200, "OK", body));
        let adapter = adapter_for(&format!("{}/v1", server.base_url), None);
        let resp = adapter.complete(&req()).unwrap();
        server.finish();
        assert_eq!(resp.stop_reason, "refusal");
        assert_eq!(resp.stop(), StopKind::Refusal);
        assert_eq!(resp.text, "");
        assert_eq!((resp.input_tokens, resp.output_tokens), (5, 8));

        // A null or empty refusal is not a refusal.
        let adapter = offline(None);
        for body in [
            r#"{"choices":[{"message":{"content":"x","refusal":null},"finish_reason":"stop"}]}"#,
            r#"{"choices":[{"message":{"content":"x","refusal":""},"finish_reason":"stop"}]}"#,
        ] {
            let resp = adapter.parse_chat_response(body).unwrap();
            assert_eq!(resp.stop_reason, "stop");
            assert_eq!(resp.stop(), StopKind::EndTurn);
        }
    }

    #[test]
    fn errors_inside_an_http_200_are_errors() {
        let key = "sk-or-VERY-SECRET-KEY";
        // Top-level error object (OpenRouter style), over HTTP, echoing the
        // key and embedding a newline.
        let top = format!(
            r#"{{"error":{{"code":402,"message":"Insufficient credits\nfor key {key}","metadata":{{}}}},"user_id":"u"}}"#
        );
        let server = serve_once(response(200, "OK", &top));
        let adapter = adapter_for(&format!("{}/v1", server.base_url), Some(key));
        let err = adapter.complete(&req()).unwrap_err().to_string();
        // An error in a 200 body is not retried.
        assert_eq!(server.finish().len(), 1);
        assert_eq!(
            err,
            "openai-compat api: error in response body: Insufficient credits for key <redacted> \
             (code 402)"
        );

        // choices[0].error, with a finish_reason of "error".
        let adapter = offline(Some(key));
        let in_choice = r#"{"choices":[{"index":0,"message":{"role":"assistant","content":""},"finish_reason":"error","error":{"code":"upstream_failed","message":"Provider returned error"}}],"usage":{"prompt_tokens":1,"completion_tokens":0}}"#;
        let err = adapter
            .parse_chat_response(in_choice)
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "openai-compat api: error in response body: Provider returned error \
             (code \"upstream_failed\")"
        );

        // finish_reason "error" alone.
        let bare = r#"{"choices":[{"message":{"content":"partial"},"finish_reason":"error"}]}"#;
        let err = adapter.parse_chat_response(bare).unwrap_err().to_string();
        assert!(
            err.starts_with("openai-compat api: error in response body: finish_reason"),
            "{err}"
        );

        // Bare-string and message-less errors are still errors, scrubbed and
        // capped like any other provider text.
        let err = adapter
            .parse_chat_response(r#"{"error":"model not found"}"#)
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "openai-compat api: error in response body: model not found"
        );
        let long = format!(
            r#"{{"error":{{"detail":"{key}","pad":"{}"}}}}"#,
            "x".repeat(2000)
        );
        let err = adapter.parse_chat_response(&long).unwrap_err().to_string();
        assert!(!err.contains("SECRET"), "{err}");
        assert!(err.contains("<redacted>"), "{err}");
        assert!(err.ends_with('…'), "{err}");

        // `"error": null` next to a good completion is not an error.
        let fine = r#"{"error":null,"choices":[{"error":null,"message":{"content":"x"},"finish_reason":"stop"}]}"#;
        assert_eq!(adapter.parse_chat_response(fine).unwrap().text, "x");
    }

    #[test]
    fn missing_or_empty_choices_and_garbage_are_errors() {
        let key = "sk-test-SECRET";
        let adapter = offline(Some(key));
        for body in [
            r#"{}"#.to_string(),
            r#"{"choices":[]}"#.to_string(),
            r#"{"choices":null}"#.to_string(),
            r#"{"choices":["not an object"]}"#.to_string(),
            format!(r#"{{"object":"list","data":[],"note":"{key}"}}"#),
        ] {
            let err = adapter.parse_chat_response(&body).unwrap_err().to_string();
            assert!(
                err.starts_with("openai-compat api: response has no choices: "),
                "{body}: {err}"
            );
            assert!(!err.contains("SECRET"), "{err}");
        }
        let err = adapter
            .parse_chat_response("<html>")
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("openai-compat api: unparseable response body"),
            "{err}"
        );
    }

    #[test]
    fn http_400_surfaces_error_body_without_the_key() {
        let key = "sk-test-VERY-SECRET-KEY";
        // A hostile/verbose server: echoes the key (also as the header it
        // arrived in), embeds newlines, and pads far past the echo cap.
        let body = format!(
            "{{\"error\":{{\"message\":\"max_tokens is too large\\nauth was Bearer {key}\",\
             \"type\":\"invalid_request_error\",\"param\":\"max_tokens\",\"code\":null}}}}\n{}",
            "x".repeat(2000)
        );
        let server = serve_once(response(400, "Bad Request", &body));
        let adapter = adapter_for(&format!("{}/v1", server.base_url), Some(key));
        let err = adapter.complete(&req()).unwrap_err().to_string();
        // Exactly one request: a 400 is not retried.
        assert_eq!(server.finish().len(), 1);

        assert!(
            err.starts_with("openai-compat api: http status 400: {"),
            "{err}"
        );
        assert!(err.contains("invalid_request_error"), "{err}");
        assert!(err.contains("max_tokens is too large"), "{err}");
        assert!(!err.contains(key), "{err}");
        assert!(!err.contains("SECRET"), "{err}");
        assert!(err.contains("Bearer <redacted>"), "{err}");
        assert!(!err.contains('\n'), "error must stay single-line: {err}");
        let shown = err
            .strip_prefix("openai-compat api: http status 400: ")
            .unwrap();
        assert_eq!(
            shown.chars().count(),
            ERROR_TEXT_MAX_CHARS + 1,
            "500 chars + ellipsis"
        );
        assert!(shown.ends_with('…'));

        // An empty error body reports the status alone.
        let server = serve_once(response(404, "Not Found", ""));
        let adapter = adapter_for(&format!("{}/v1", server.base_url), None);
        let err = adapter.complete(&req()).unwrap_err().to_string();
        server.finish();
        assert_eq!(err, "openai-compat api: http status 404");
    }

    #[test]
    fn retries_once_on_429_then_succeeds() {
        let server = serve(vec![
            response(
                429,
                "Too Many Requests",
                r#"{"error":{"message":"slow down"}}"#,
            ),
            response(200, "OK", OK_BODY),
        ]);
        let adapter = adapter_for(&format!("{}/v1", server.base_url), Some("sk-test-SECRET"));
        let resp = adapter.complete(&req()).unwrap();
        assert_eq!(resp.text, "hello");
        assert_eq!((resp.input_tokens, resp.output_tokens), (3, 1));
        assert_eq!(resp.stop(), StopKind::EndTurn);
        let seen = server.finish();
        assert_eq!(seen.len(), 2, "one call + one retry");
        assert_eq!(seen[0].request_line, seen[1].request_line);
        assert_eq!(
            seen[0].body, seen[1].body,
            "the retry resends the same body"
        );
        assert_eq!(
            seen[1].header("authorization"),
            Some("Bearer sk-test-SECRET")
        );
    }

    #[test]
    fn retry_is_single_and_reports_the_second_failure() {
        let server = serve(vec![
            response(500, "Internal Server Error", "first"),
            response(503, "Service Unavailable", "overloaded"),
        ]);
        let adapter = adapter_for(&format!("{}/v1", server.base_url), None);
        let err = adapter.complete(&req()).unwrap_err().to_string();
        assert_eq!(
            server.finish().len(),
            2,
            "exactly one retry, never a third call"
        );
        assert_eq!(
            err,
            "openai-compat api (after retry): http status 503: overloaded"
        );

        assert_eq!(
            from_profile(&profile("http://127.0.0.1:1/v1"), &no_env)
                .unwrap()
                .retry_delay,
            Duration::from_secs(2),
            "default pause"
        );
        assert!(retryable(429) && retryable(500) && retryable(529) && retryable(599));
        assert!(!retryable(200) && !retryable(400) && !retryable(401) && !retryable(302));
    }

    #[test]
    fn redirects_are_never_followed() {
        // Following the redirect would need a second connection; the mock
        // serves exactly one, and the 302 itself must be what is reported.
        let server = serve_once(response_with_headers(
            302,
            "Found",
            &[("location", "http://127.0.0.1:9/elsewhere")],
            "",
        ));
        let adapter = adapter_for(&format!("{}/v1", server.base_url), Some("sk-test-SECRET"));
        let err = adapter.complete(&req()).unwrap_err().to_string();
        assert_eq!(server.finish().len(), 1);
        assert_eq!(err, "openai-compat api: http status 302");
    }

    #[test]
    fn timeout_is_explicit_and_enforced() {
        // 600s unless the profile says otherwise.
        let mut unhurried = profile("http://127.0.0.1:1/v1");
        unhurried.timeout_secs = None;
        assert_eq!(
            from_profile(&unhurried, &no_env).unwrap().timeout,
            Duration::from_secs(600)
        );

        // A server that never answers: the call must end at ~timeout_secs.
        let server = serve_once(Reply::Hang);
        let mut slow = profile(&format!("{}/v1", server.base_url));
        slow.timeout_secs = Some(1);
        let adapter = from_profile(&slow, &no_env).unwrap();
        let started = Instant::now();
        let err = adapter.complete(&req()).unwrap_err().to_string();
        let elapsed = started.elapsed();
        drop(adapter);
        server.finish();
        assert!(
            err.starts_with("openai-compat api: timed out after 1s"),
            "{err}"
        );
        assert!(err.contains("timeout_secs"), "{err}");
        assert!(elapsed < Duration::from_secs(8), "took {elapsed:?}");
    }

    #[test]
    fn transport_errors_are_reported_without_retry_or_key() {
        // Nothing listens on this port any more.
        let server = serve(vec![]);
        let base_url = server.base_url.clone();
        server.finish();
        let adapter = adapter_for(&format!("{base_url}/v1"), Some("sk-test-SECRET"));
        let err = adapter.complete(&req()).unwrap_err().to_string();
        assert!(err.starts_with("openai-compat api: "), "{err}");
        assert!(!err.contains("after retry"), "{err}");
        assert!(!err.contains("SECRET"), "{err}");
    }

    // ---- construction ------------------------------------------------------

    #[test]
    fn debug_never_shows_the_key() {
        let adapter = offline(Some("sk-test-SECRET"));
        assert_eq!(adapter.api_key.as_deref(), Some("sk-test-SECRET"));
        let rendered = format!("{adapter:?}");
        assert!(!rendered.contains("SECRET"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(rendered.contains("http://127.0.0.1:1/v1"), "{rendered}");
        assert!(rendered.contains("30s"), "{rendered}");

        let rendered = format!("{:?}", offline(None));
        assert!(rendered.contains("api_key: None"), "{rendered}");
    }

    #[test]
    fn build_yields_the_openai_compat_adapter() {
        let adapter = build(&profile("http://127.0.0.1:1/v1"), &no_env).unwrap();
        assert_eq!(adapter.name(), "openai-compat");
    }

    #[test]
    fn profile_errors_name_the_variable_and_never_echo_a_pasted_key() {
        // Named but unset / empty / not UTF-8 → the `set …` hint.
        let mut keyed = profile("https://gateway.example/v1");
        keyed.api_key_env = Some("MY_GATEWAY_TOKEN".into());
        #[cfg(unix)]
        let not_utf8 = {
            use std::os::unix::ffi::OsStringExt;
            Some(OsString::from_vec(vec![0xff, 0xfe]))
        };
        #[cfg(not(unix))]
        let not_utf8: Option<OsString> = None;
        for value in [None, Some(OsString::new()), not_utf8] {
            let err = from_profile(&keyed, &|_| value.clone())
                .unwrap_err()
                .to_string();
            assert_eq!(err, "set MY_GATEWAY_TOKEN to use the local-openai provider");
        }

        // A value that is not shaped like a variable name — typically the
        // key itself, pasted into the wrong field — is refused unread and
        // is NOT echoed.
        for pasted in ["sk-proj-SECRET", "", "A=B", "MY KEY", "1ST", "K\n"] {
            let mut bad = profile("https://gateway.example/v1");
            bad.api_key_env = Some(pasted.into());
            let err = from_profile(&bad, &|_| panic!("lookup must not run for a refused name"))
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("local-openai") && err.contains("api_key_env"),
                "{err}"
            );
            assert!(!err.contains("SECRET") && !err.contains('\n'), "{err}");
        }

        let mut zero = profile("https://gateway.example/v1");
        zero.timeout_secs = Some(0);
        let err = from_profile(&zero, &no_env).unwrap_err().to_string();
        assert!(
            err.contains("local-openai") && err.contains("timeout_secs"),
            "{err}"
        );

        // Only the two schema spellings can become a request-body key.
        let mut odd = profile("https://gateway.example/v1");
        odd.max_tokens_field = Some("n_predict\",\"temperature\":2,\"x".into());
        let err = from_profile(&odd, &no_env).unwrap_err().to_string();
        assert!(
            err.contains("max_tokens_field is not one of: max_tokens, max_completion_tokens"),
            "{err}"
        );
        assert!(!err.contains("n_predict"), "{err}");
    }

    // ---- registration: the one arm in `kind_builder` -------------------------

    #[test]
    fn a_user_profile_of_this_kind_resolves_end_to_end() {
        let server = serve_once(response(200, "OK", OK_BODY));
        let dir = std::env::temp_dir().join(format!(
            "harness-llm-openai-compat-resolve-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("providers.toml");
        std::fs::write(
            &path,
            format!(
                "[providers.local-openai]\nkind = \"openai-compat\"\nbase_url = \"{}/v1\"\n\
                 api_key_env = \"RUHARNESS_TEST_OPENAI_COMPAT_TOKEN\"\ncontext_tokens = 8192\n\
                 max_tokens_field = \"max_completion_tokens\"\ntimeout_secs = 30\n",
                server.base_url
            ),
        )
        .unwrap();
        let _env = EnvGuard::set(&[
            (crate::providers::PROFILES_ENV, path.to_str().unwrap()),
            ("RUHARNESS_TEST_OPENAI_COMPAT_TOKEN", "tok-from-env-SECRET"),
        ]);

        let got = crate::providers::resolve("local-openai", Path::new("/unused")).unwrap();
        assert_eq!(got.profile, "local-openai");
        assert_eq!(got.kind, "openai-compat");
        assert_eq!(got.adapter.name(), "openai-compat");
        assert_eq!(got.context_tokens, Some(8192));
        assert!(got.live);
        assert!(!format!("{got:?}").contains("SECRET"));

        let resp = crate::providers::checked_complete(&got, &req()).unwrap();
        assert_eq!(resp.text, "hello");
        let seen = server.finish();
        assert_eq!(seen[0].request_line, "POST /v1/chat/completions HTTP/1.1");
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer tok-from-env-SECRET")
        );
        let body: Value = serde_json::from_str(&seen[0].body).unwrap();
        assert_eq!(
            body_keys(&body),
            ["max_completion_tokens", "messages", "model"]
        );
    }
}
