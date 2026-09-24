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

/// Default end-to-end timeout of one live call, in seconds (docs/SCHEMAS.md
/// "Provider profiles": `timeout_secs`). ureq 3 has NO default timeouts, so
/// every adapter sets one explicitly.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// The `anthropic-version` header value this adapter speaks.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Longest prefix of a provider error body echoed in an error, in chars.
const ERROR_BODY_MAX_CHARS: usize = 500;

/// Pause before the single retry on 429/5xx.
const RETRY_DELAY: Duration = Duration::from_secs(2);

/// Live adapter for the Anthropic Messages API (and servers that speak it,
/// reached through a user-level provider profile — see
/// [`crate::providers`]). Model-agnostic: the model id travels on each
/// [`CompletionRequest`], and only ever inside the request body.
///
/// The API key is optional (a local server needs none); when present it is
/// held only in memory and sent only as the `x-api-key` header. It never
/// appears in any error message, log, or file: the [`std::fmt::Debug`]
/// rendering redacts it, every error string built from provider output is
/// scrubbed of it, and traces are recorded from the provider-neutral
/// request/response types, which do not carry it. Redirects are never
/// followed, so the header cannot be forwarded to another host.
pub struct AnthropicAdapter {
    api_key: Option<String>,
    base_url: String,
    timeout: Duration,
    retry_delay: Duration,
    agent: ureq::Agent,
}

impl std::fmt::Debug for AnthropicAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicAdapter")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl AnthropicAdapter {
    /// Adapter with the given key, the default base URL and the default
    /// timeout ([`DEFAULT_TIMEOUT_SECS`]).
    pub fn new(api_key: impl Into<String>) -> AnthropicAdapter {
        Self::build(
            Some(api_key.into()),
            ANTHROPIC_DEFAULT_BASE_URL,
            DEFAULT_TIMEOUT_SECS,
        )
    }

    /// Adapter with an explicit base URL (trailing slashes trimmed) and the
    /// default timeout.
    pub fn with_base_url(api_key: impl Into<String>, base_url: &str) -> AnthropicAdapter {
        Self::build(Some(api_key.into()), base_url, DEFAULT_TIMEOUT_SECS)
    }

    /// The one real constructor: builds the ureq agent with an explicit
    /// global timeout (ureq 3 has none by default), HTTP error statuses
    /// returned as responses (so error bodies can be read), and redirects
    /// disabled (ureq only strips `Authorization` on redirect — an
    /// `x-api-key` header would follow a redirect to any host). An empty key
    /// counts as no key. A loopback endpoint bypasses any `*_PROXY`
    /// environment proxy (ureq's default): a proxy cannot reach the user's
    /// own loopback, and a local server's traffic should never leave the
    /// machine.
    fn build(api_key: Option<String>, base_url: &str, timeout_secs: u64) -> AnthropicAdapter {
        let timeout = Duration::from_secs(timeout_secs);
        let mut config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .max_redirects(0);
        if is_loopback_url(base_url) {
            config = config.proxy(None);
        }
        let agent: ureq::Agent = config.build().into();
        AnthropicAdapter {
            api_key: api_key.filter(|k| !k.is_empty()),
            base_url: base_url.trim_end_matches('/').to_string(),
            timeout,
            retry_delay: RETRY_DELAY,
            agent,
        }
    }

    /// The BUILT-IN `anthropic` profile's constructor: default base URL,
    /// default timeout, key read from the environment variable
    /// `api_key_env`. An unset (or empty) variable is an error telling the
    /// user to set it.
    ///
    /// Only names matching `^ANTHROPIC_[A-Za-z0-9_]*$` are accepted here.
    /// The built-in profile pairs the variable with `api.anthropic.com`, so
    /// whatever selects the name must never be able to point it at an
    /// unrelated secret (`AWS_SECRET_ACCESS_KEY`, `GITHUB_TOKEN`, …) and
    /// have it sent as the `x-api-key` header (threat model §12.1; at M2 the
    /// name came from the target's hostile `harness.toml`). User-level
    /// profiles are user-owned and go through [`Self::from_profile`], which
    /// allows any variable name.
    pub fn from_env(api_key_env: &str) -> Result<AnthropicAdapter, Error> {
        Self::from_env_value(api_key_env, |name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] with the environment lookup injected (so the
    /// policy is testable without mutating the process environment). The
    /// name check runs BEFORE the lookup: a refused name is never read.
    pub(crate) fn from_env_value(
        api_key_env: &str,
        lookup: impl FnOnce(&str) -> Option<String>,
    ) -> Result<AnthropicAdapter, Error> {
        if !is_allowed_api_key_env(api_key_env) {
            return Err(Error::Invariant(format!(
                "api_key_env {api_key_env:?} is not allowed: the built-in anthropic provider                  only reads variables named ANTHROPIC_* (it must not be possible to select                  which secret is sent as the api key)"
            )));
        }
        match lookup(api_key_env) {
            Some(key) if !key.is_empty() => Ok(AnthropicAdapter::new(key)),
            _ => Err(Error::Invariant(format!(
                "set {api_key_env} to use the anthropic provider"
            ))),
        }
    }

    /// Constructor from a USER-LEVEL provider profile (docs/SCHEMAS.md
    /// "Provider profiles"): `profile` is the profile's name (used only in
    /// error messages), `base_url` its endpoint (trailing slashes trimmed),
    /// `api_key_env` the optional environment variable holding the key
    /// (`None` = no auth header is sent), `timeout_secs` the end-to-end
    /// timeout of one call (`None` = [`DEFAULT_TIMEOUT_SECS`]; `0` is an
    /// error).
    ///
    /// Profiles live in user-owned config, never in the target, so ANY
    /// variable name is allowed here — the `ANTHROPIC_*` restriction applies
    /// only to the built-in profile ([`Self::from_env`]). The name must only
    /// LOOK like a variable name (`^[A-Za-z_][A-Za-z0-9_]*$`): a value that
    /// does not is refused without being echoed, because the usual way to
    /// get one is pasting the key itself into `api_key_env`. A named
    /// variable that is unset or empty is an error telling the user to set
    /// it.
    pub fn from_profile(
        profile: &str,
        base_url: &str,
        api_key_env: Option<&str>,
        timeout_secs: Option<u64>,
    ) -> Result<AnthropicAdapter, Error> {
        Self::from_profile_with(profile, base_url, api_key_env, timeout_secs, |name| {
            std::env::var(name).ok()
        })
    }

    /// [`Self::from_profile`] with the environment lookup injected. The
    /// lookup runs only when the profile names a variable.
    pub(crate) fn from_profile_with(
        profile: &str,
        base_url: &str,
        api_key_env: Option<&str>,
        timeout_secs: Option<u64>,
        lookup: impl FnOnce(&str) -> Option<String>,
    ) -> Result<AnthropicAdapter, Error> {
        let timeout_secs = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        if timeout_secs == 0 {
            return Err(Error::Invariant(format!(
                "provider profile `{profile}`: timeout_secs must be greater than 0"
            )));
        }
        let api_key = match api_key_env {
            None => None,
            Some(name) if !is_env_var_name(name) => {
                // Deliberately not echoed: the classic mistake is pasting
                // the key itself here, and errors end up in logs.
                return Err(Error::Invariant(format!(
                    "provider profile `{profile}`: api_key_env is not an environment variable \
                     name (expected ^[A-Za-z_][A-Za-z0-9_]*$; value not shown) — it names the \
                     variable that holds the key, never the key itself"
                )));
            }
            Some(name) => match lookup(name) {
                Some(key) if !key.is_empty() => Some(key),
                _ => {
                    return Err(Error::Invariant(format!(
                        "set {name} to use the {profile} provider"
                    )))
                }
            },
        };
        Ok(Self::build(api_key, base_url, timeout_secs))
    }

    /// The end-to-end timeout applied to every call.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// One POST to `/v1/messages`, returning the HTTP status and the raw
    /// response body text. `x-api-key` is sent only when a key is
    /// configured. For an error status an unreadable body degrades to an
    /// empty one rather than masking the status.
    fn send_once(&self, url: &str, body: &str) -> Result<(u16, String), ureq::Error> {
        let mut request = self
            .agent
            .post(url)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json");
        if let Some(key) = &self.api_key {
            request = request.header("x-api-key", key);
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
        Error::Invariant(self.scrub(&format!("anthropic api{phase}: {detail}")))
    }

    /// Error for a non-success HTTP status, surfacing the provider's error
    /// body: api key scrubbed FIRST, control characters flattened to spaces
    /// (the message stays single-line), trimmed, then capped at
    /// [`ERROR_BODY_MAX_CHARS`] chars.
    fn status_error(&self, phase: &str, status: u16, body: &str) -> Error {
        let flattened: String = self
            .scrub(body)
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let flattened = flattened.trim();
        let mut shown: String = flattened.chars().take(ERROR_BODY_MAX_CHARS).collect();
        if shown.len() < flattened.len() {
            shown.push('…');
        }
        Error::Invariant(if shown.is_empty() {
            format!("anthropic api{phase}: http status {status}")
        } else {
            format!("anthropic api{phase}: http status {status}: {shown}")
        })
    }
}

/// The built-in profile's `api_key_env` policy: `^ANTHROPIC_[A-Za-z0-9_]*$`.
fn is_allowed_api_key_env(name: &str) -> bool {
    name.strip_prefix("ANTHROPIC_")
        .is_some_and(|rest| rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

/// `^[A-Za-z_][A-Za-z0-9_]*$` — the portable shape of an environment
/// variable name.
pub(crate) fn is_env_var_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// True when `base_url`'s host is `localhost` or a loopback IP literal
/// (`127.0.0.0/8`, `[::1]`). Purely textual — no name resolution.
pub(crate) fn is_loopback_url(base_url: &str) -> bool {
    let rest = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host_port)| host_port);
    let host = match host_port.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host_port
            .rsplit_once(':')
            .map_or(host_port, |(host, _)| host),
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// True for statuses worth one retry (rate limit / server-side failure).
fn retryable(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

impl ProviderAdapter for AnthropicAdapter {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    /// POST `{base}/v1/messages` with exactly `model`, `max_tokens`,
    /// `system`, `messages` — adapters never send sampling or thinking
    /// parameters (docs/SCHEMAS.md). One retry after a 2s pause on 429/5xx;
    /// any other non-2xx status is an error carrying the (scrubbed,
    /// truncated) provider error body.
    ///
    /// The provider's `stop_reason` is returned RAW and is never an error
    /// here (`max_tokens`, `refusal`, … included): callers decide via
    /// [`CompletionResponse::stop`].
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
        parse_messages_response(&text)
    }
}

/// Parse a Messages API response body into a [`CompletionResponse`]:
/// concatenated `text` content blocks, usage accounting, and the RAW
/// `stop_reason` string (absent/null → `""`). No stop reason is an error at
/// this layer.
fn parse_messages_response(body: &str) -> Result<CompletionResponse, Error> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| Error::Invariant(format!("anthropic api: unparseable response body: {e}")))?;
    let stop_reason = v
        .get("stop_reason")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
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

/// Largest trace file [`TraceAdapter::load_recorded`] reads.
pub const MAX_TRACE_BYTES: u64 = 16 * 1024 * 1024;

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

    /// Read the RECORDED request/response pair stored under `key` in `dir`
    /// — the evidence-first replay's only way to a recorded turn
    /// (docs/REPLAY-DESIGN.md §R R-2). Read-only: never writes, never files
    /// a hand-off. `key` must be exactly 8 lowercase hex digits before it
    /// becomes a path component; the dir and both files must be real
    /// (non-symlink) entries of bounded size; the request must re-serialize
    /// to `key`.
    pub fn load_recorded(
        dir: &Path,
        key: &str,
    ) -> Result<(CompletionRequest, CompletionResponse), Error> {
        if key.len() != 8
            || !key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invariant(format!(
                "recorded request key {:?} is not 8 lowercase hex digits",
                key.chars().take(16).collect::<String>()
            )));
        }
        let regular = |path: &Path| -> Result<(), Error> {
            let meta = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
            let is_dir = path == dir && meta.file_type().is_dir();
            if !(meta.file_type().is_file() || is_dir) {
                return Err(Error::Invariant(format!(
                    "{} is not a regular file or directory (symlinks are refused)",
                    path.display()
                )));
            }
            if meta.file_type().is_file() && meta.len() > MAX_TRACE_BYTES {
                return Err(Error::Invariant(format!(
                    "{} exceeds {MAX_TRACE_BYTES} bytes",
                    path.display()
                )));
            }
            Ok(())
        };
        regular(dir)?;
        let read = |ext: &str| -> Result<(PathBuf, String), Error> {
            let path = dir.join(format!("{key}.{ext}.json"));
            regular(&path)?;
            let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
            Ok((path, text))
        };
        let (req_path, req_text) = read("request")?;
        let request: CompletionRequest =
            serde_json::from_str(&req_text).map_err(|e| Error::parse(&req_path, e.to_string()))?;
        let actual = Self::request_key(&request)?;
        if actual != key {
            return Err(Error::Invariant(format!(
                "{} hashes to request key {actual}, not {key}: the recorded request was altered",
                req_path.display()
            )));
        }
        let (resp_path, resp_text) = read("response")?;
        let response: CompletionResponse = serde_json::from_str(&resp_text)
            .map_err(|e| Error::parse(&resp_path, e.to_string()))?;
        Ok((request, response))
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
            // Typed (docs/CLI-HARDENING.md §4): the trajectory fills in the
            // attempt id; the CLI maps it to its `awaiting` event.
            Err(Error::Awaiting {
                path: response_path,
                attempt: None,
            })
        } else {
            Err(Error::Invariant(format!(
                "missing trace {} — record a live run first or use the external provider",
                response_path.display()
            )))
        }
    }
}

/// Test-only one-shot mock HTTP server on `127.0.0.1:0` (no network beyond
/// localhost). Shared by every adapter's tests in this crate.
#[cfg(test)]
pub(crate) mod mock_http {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    /// How long the server waits for each expected connection.
    const ACCEPT_DEADLINE: Duration = Duration::from_secs(10);

    /// One captured HTTP request.
    #[derive(Debug, Clone)]
    pub(crate) struct Captured {
        /// E.g. `POST /v1/messages HTTP/1.1`.
        pub(crate) request_line: String,
        /// `(lower-cased name, trimmed value)` pairs in wire order.
        pub(crate) headers: Vec<(String, String)>,
        /// The full body (read by `Content-Length`).
        pub(crate) body: String,
    }

    impl Captured {
        /// First value of header `name` (case-insensitive).
        pub(crate) fn header(&self, name: &str) -> Option<&str> {
            let name = name.to_ascii_lowercase();
            self.headers
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.as_str())
        }
    }

    /// What the server does with one connection after reading its request.
    pub(crate) enum Reply {
        /// Write these raw bytes (a full HTTP/1.1 response) and close.
        Canned(String),
        /// Never answer: hold the connection until the client gives up.
        Hang,
    }

    /// A canned `HTTP/1.1` JSON response. `connection: close` forces the
    /// client to open a fresh connection for any follow-up request, so each
    /// [`Reply`] maps to exactly one request.
    pub(crate) fn response(status: u16, reason: &str, body: &str) -> Reply {
        response_with_headers(status, reason, &[], body)
    }

    /// [`response`] with extra response headers.
    pub(crate) fn response_with_headers(
        status: u16,
        reason: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Reply {
        let mut head = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        Reply::Canned(format!("{head}\r\n{body}"))
    }

    /// A running mock server; [`MockServer::finish`] hands back what it saw.
    pub(crate) struct MockServer {
        /// `http://127.0.0.1:<port>`.
        pub(crate) base_url: String,
        handle: JoinHandle<Vec<Captured>>,
    }

    impl MockServer {
        /// Wait for the server to have served every reply and return the
        /// captured requests in order. Panics (failing the test) when fewer
        /// connections arrived than replies were queued.
        pub(crate) fn finish(self) -> Vec<Captured> {
            self.handle.join().expect("mock server thread panicked")
        }
    }

    /// Serve exactly one connection.
    pub(crate) fn serve_once(reply: Reply) -> MockServer {
        serve(vec![reply])
    }

    /// Serve exactly `replies.len()` connections, sequentially, one reply
    /// each, then stop listening (a further connection is refused).
    pub(crate) fn serve(replies: Vec<Reply>) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
        let port = listener.local_addr().expect("local_addr").port();
        listener.set_nonblocking(true).expect("set_nonblocking");
        let handle = std::thread::spawn(move || {
            let mut captured = Vec::new();
            for reply in replies {
                let mut stream = accept_with_deadline(&listener);
                let request = read_request(&mut stream);
                captured.push(request);
                match reply {
                    Reply::Canned(bytes) => {
                        stream.write_all(bytes.as_bytes()).expect("write response");
                        stream.flush().expect("flush response");
                    }
                    Reply::Hang => {
                        // Block until the client hangs up (EOF or error) or
                        // the read timeout fires; never write anything.
                        let mut sink = [0u8; 64];
                        while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
                    }
                }
            }
            captured
        });
        MockServer {
            base_url: format!("http://127.0.0.1:{port}"),
            handle,
        }
    }

    fn accept_with_deadline(listener: &TcpListener) -> TcpStream {
        let deadline = Instant::now() + ACCEPT_DEADLINE;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    // Accepted sockets inherit O_NONBLOCK on some platforms.
                    stream.set_nonblocking(false).expect("blocking stream");
                    stream
                        .set_read_timeout(Some(ACCEPT_DEADLINE))
                        .expect("read timeout");
                    return stream;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "mock server: expected another connection within {ACCEPT_DEADLINE:?}"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("mock server: accept failed: {e}"),
            }
        }
    }

    /// Read one full request: head up to the blank line, then exactly
    /// `Content-Length` body bytes.
    fn read_request(stream: &mut TcpStream) -> Captured {
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            let n = stream.read(&mut chunk).expect("mock server: read head");
            assert!(n > 0, "mock server: connection closed inside the head");
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8(buf[..head_end].to_vec()).expect("utf-8 request head");
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap_or_default().to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let header = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        };
        if header("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
            stream
                .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
                .expect("write 100-continue");
        }
        let content_length: usize = header("content-length")
            .expect("mock server: request has no content-length")
            .parse()
            .expect("numeric content-length");
        let mut body = buf[head_end + 4..].to_vec();
        while body.len() < content_length {
            let n = stream.read(&mut chunk).expect("mock server: read body");
            assert!(n > 0, "mock server: connection closed inside the body");
            body.extend_from_slice(&chunk[..n]);
        }
        assert_eq!(
            body.len(),
            content_length,
            "body longer than content-length"
        );
        Captured {
            request_line,
            headers,
            body: String::from_utf8(body).expect("utf-8 request body"),
        }
    }
}

/// Test-only process-environment mutation, serialized crate-wide.
#[cfg(test)]
pub(crate) mod test_env {
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard};

    /// Serializes every test that mutates the process environment.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Sets variables for the guard's lifetime while holding the crate-wide
    /// environment lock; the previous values are restored on drop.
    pub(crate) struct EnvGuard {
        previous: Vec<(String, Option<OsString>)>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        /// Take the lock, then set every `(name, value)` pair.
        pub(crate) fn set(vars: &[(&str, &str)]) -> EnvGuard {
            let lock = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = vars
                .iter()
                .map(|(name, value)| {
                    let old = std::env::var_os(name);
                    std::env::set_var(name, value);
                    (name.to_string(), old)
                })
                .collect();
            EnvGuard {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, old) in &self.previous {
                match old {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock_http::{response, response_with_headers, serve, serve_once, Reply};
    use super::test_env::EnvGuard;
    use super::*;
    use harness_core::traits::StopKind;
    use std::time::Instant;

    const OK_BODY: &str = r#"{"content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":3,"output_tokens":1},"stop_reason":"end_turn"}"#;

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

    /// An adapter aimed at a mock server, with a near-zero retry pause.
    fn adapter_for(base_url: &str, api_key: Option<&str>) -> AnthropicAdapter {
        let mut adapter = AnthropicAdapter::build(api_key.map(str::to_string), base_url, 30);
        adapter.retry_delay = Duration::from_millis(10);
        adapter
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

    /// Regression: the trace format is unchanged from M2 — a response file
    /// written by hand in the M2 shape (exactly these four keys, in this
    /// order) still loads, in both external and replay mode.
    #[test]
    fn m2_shaped_response_file_still_loads() {
        let dir = temp_dir("m2-shape");
        let request = req();
        let m2 = r#"{"text":"[]","input_tokens":0,"output_tokens":0,"stop_reason":"end_turn"}"#;
        std::fs::write(TraceAdapter::response_path(&dir, &request).unwrap(), m2).unwrap();
        for external in [true, false] {
            let got = TraceAdapter::new(&dir, external)
                .complete(&request)
                .unwrap();
            assert_eq!(got.text, "[]");
            assert_eq!((got.input_tokens, got.output_tokens), (0, 0));
            assert_eq!(got.stop_reason, "end_turn");
            assert_eq!(got.stop(), StopKind::EndTurn);
        }
        // … and what `record` writes today is still exactly that shape.
        let recorded_dir = temp_dir("m2-shape-recorded");
        let resp: CompletionResponse = serde_json::from_str(m2).unwrap();
        TraceAdapter::record(&recorded_dir, &request, &resp).unwrap();
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(TraceAdapter::response_path(&recorded_dir, &request).unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            on_disk,
            serde_json::from_str::<serde_json::Value>(m2).unwrap()
        );
    }

    #[test]
    fn messages_response_parses_and_passes_stop_reason_through() {
        let ok = r#"{"content":[{"type":"text","text":"a"},{"type":"tool_use","id":"x"},{"type":"text","text":"b"}],"usage":{"input_tokens":10,"output_tokens":4},"stop_reason":"end_turn"}"#;
        let resp = parse_messages_response(ok).unwrap();
        assert_eq!(resp.text, "ab");
        assert_eq!((resp.input_tokens, resp.output_tokens), (10, 4));
        assert_eq!(resp.stop_reason, "end_turn");
        assert_eq!(resp.stop(), StopKind::EndTurn);

        // Truncation and refusal are NOT errors at the adapter layer any
        // more: the raw string comes back and callers gate on `stop()`.
        let truncated =
            r#"{"content":[{"type":"text","text":"a"}],"usage":{},"stop_reason":"max_tokens"}"#;
        let resp = parse_messages_response(truncated).unwrap();
        assert_eq!(
            (resp.text.as_str(), resp.stop_reason.as_str()),
            ("a", "max_tokens")
        );
        assert_eq!(resp.stop(), StopKind::MaxTokens);
        assert_eq!((resp.input_tokens, resp.output_tokens), (0, 0));

        let refused = r#"{"content":[],"usage":{},"stop_reason":"refusal"}"#;
        let resp = parse_messages_response(refused).unwrap();
        assert_eq!(
            (resp.text.as_str(), resp.stop_reason.as_str()),
            ("", "refusal")
        );
        assert_eq!(resp.stop(), StopKind::Refusal);

        // Unknown and absent stop reasons pass through raw as well.
        let odd = r#"{"content":[],"stop_reason":"pause_turn"}"#;
        assert_eq!(
            parse_messages_response(odd).unwrap().stop_reason,
            "pause_turn"
        );
        let absent = r#"{"content":[],"stop_reason":null}"#;
        let resp = parse_messages_response(absent).unwrap();
        assert_eq!(resp.stop_reason, "");
        assert_eq!(resp.stop(), StopKind::Other);

        let err = parse_messages_response("<html>").unwrap_err().to_string();
        assert!(err.contains("unparseable response body"), "{err}");
    }

    #[test]
    fn request_wire_shape_with_api_key() {
        let server = serve_once(response(200, "OK", OK_BODY));
        let adapter = adapter_for(&format!("{}/", server.base_url), Some("sk-ant-SECRET"));
        let resp = adapter.complete(&req()).unwrap();
        assert_eq!(resp.text, "hello");

        let seen = server.finish();
        assert_eq!(seen.len(), 1);
        let got = &seen[0];
        // Trailing slash on the base URL is trimmed: no `//v1/messages`.
        assert_eq!(got.request_line, "POST /v1/messages HTTP/1.1");
        assert_eq!(got.header("anthropic-version"), Some("2023-06-01"));
        assert_eq!(got.header("x-api-key"), Some("sk-ant-SECRET"));
        assert_eq!(got.header("content-type"), Some("application/json"));
        assert_eq!(got.header("authorization"), None);

        // Body: exactly model/max_tokens/system/messages — no temperature,
        // no thinking, no other sampling parameter.
        let body: serde_json::Value = serde_json::from_str(&got.body).unwrap();
        let mut keys: Vec<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["max_tokens", "messages", "model", "system"]);
        assert_eq!(
            body,
            serde_json::json!({
                "model": "claude-sonnet-5",
                "max_tokens": 128,
                "system": "system",
                "messages": [{"role": "user", "content": "user"}],
            })
        );
        // The key travels only in its header.
        assert!(!got.body.contains("SECRET"));
        assert!(!got.request_line.contains("SECRET"));
    }

    #[test]
    fn no_api_key_means_no_auth_header() {
        for key in [None, Some("")] {
            let server = serve_once(response(200, "OK", OK_BODY));
            let adapter = adapter_for(&server.base_url, key);
            adapter.complete(&req()).unwrap();
            let seen = server.finish();
            assert_eq!(seen[0].header("x-api-key"), None, "key {key:?}");
            assert_eq!(seen[0].header("authorization"), None);
            assert_eq!(seen[0].header("anthropic-version"), Some("2023-06-01"));
        }
    }

    #[test]
    fn response_parsing_over_http() {
        let body = r#"{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"part one, "},{"type":"thinking","thinking":"hidden"},{"type":"text","text":"part two"}],"stop_reason":"max_tokens","usage":{"input_tokens":9120,"output_tokens":1400}}"#;
        let server = serve_once(response(200, "OK", body));
        let adapter = adapter_for(&server.base_url, Some("k"));
        // `max_tokens` is a successful completion at this layer …
        let resp = adapter.complete(&req()).unwrap();
        server.finish();
        assert_eq!(resp.text, "part one, part two");
        assert_eq!((resp.input_tokens, resp.output_tokens), (9120, 1400));
        // … with the RAW provider string preserved and the kind derived.
        assert_eq!(resp.stop_reason, "max_tokens");
        assert_eq!(resp.stop(), StopKind::MaxTokens);
    }

    #[test]
    fn http_400_surfaces_error_body_without_the_key() {
        let key = "sk-ant-VERY-SECRET-KEY";
        // A hostile/verbose server: echoes the key, embeds newlines, and
        // pads far past the echo cap.
        let body = format!(
            "{{\"type\":\"error\",\"error\":{{\"type\":\"invalid_request_error\",\
             \"message\":\"max_tokens: too large\\nkey was {key}\"}}}}\n{}",
            "x".repeat(2000)
        );
        let server = serve_once(response(400, "Bad Request", &body));
        let adapter = adapter_for(&server.base_url, Some(key));
        let err = adapter.complete(&req()).unwrap_err().to_string();
        // Exactly one request: a 400 is not retried.
        assert_eq!(server.finish().len(), 1);

        assert!(
            err.starts_with("anthropic api: http status 400: {"),
            "{err}"
        );
        assert!(err.contains("invalid_request_error"), "{err}");
        assert!(err.contains("max_tokens: too large"), "{err}");
        assert!(!err.contains(key), "{err}");
        assert!(!err.contains("SECRET"), "{err}");
        assert!(err.contains("<redacted>"), "{err}");
        assert!(!err.contains('\n'), "error must stay single-line: {err}");
        let shown = err
            .strip_prefix("anthropic api: http status 400: ")
            .unwrap();
        assert_eq!(
            shown.chars().count(),
            ERROR_BODY_MAX_CHARS + 1,
            "500 chars + ellipsis"
        );
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn error_status_with_empty_body_reports_the_status_alone() {
        let server = serve_once(response(404, "Not Found", ""));
        let adapter = adapter_for(&server.base_url, None);
        let err = adapter.complete(&req()).unwrap_err().to_string();
        server.finish();
        assert_eq!(err, "anthropic api: http status 404");
    }

    #[test]
    fn retries_once_on_429_then_succeeds() {
        let server = serve(vec![
            response(429, "Too Many Requests", r#"{"type":"error"}"#),
            response(200, "OK", OK_BODY),
        ]);
        let adapter = adapter_for(&server.base_url, Some("k"));
        let resp = adapter.complete(&req()).unwrap();
        assert_eq!(resp.text, "hello");
        assert_eq!((resp.input_tokens, resp.output_tokens), (3, 1));
        let seen = server.finish();
        assert_eq!(seen.len(), 2, "one call + one retry");
        assert_eq!(seen[0].request_line, seen[1].request_line);
        assert_eq!(
            seen[0].body, seen[1].body,
            "the retry resends the same body"
        );
        assert_eq!(seen[1].header("x-api-key"), Some("k"));
    }

    #[test]
    fn retry_is_single_and_reports_the_second_failure() {
        let server = serve(vec![
            response(500, "Internal Server Error", "first"),
            response(503, "Service Unavailable", "overloaded"),
        ]);
        let adapter = adapter_for(&server.base_url, Some("k"));
        let err = adapter.complete(&req()).unwrap_err().to_string();
        assert_eq!(
            server.finish().len(),
            2,
            "exactly one retry, never a third call"
        );
        assert_eq!(
            err,
            "anthropic api (after retry): http status 503: overloaded"
        );
    }

    #[test]
    fn default_retry_pause_is_two_seconds() {
        assert_eq!(
            AnthropicAdapter::new("k").retry_delay,
            Duration::from_secs(2)
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
        let adapter = adapter_for(&server.base_url, Some("sk-ant-SECRET"));
        let err = adapter.complete(&req()).unwrap_err().to_string();
        assert_eq!(server.finish().len(), 1);
        assert_eq!(err, "anthropic api: http status 302");
    }

    #[test]
    fn timeout_is_explicit_and_enforced() {
        // Defaults: 600s unless the profile says otherwise.
        assert_eq!(DEFAULT_TIMEOUT_SECS, 600);
        assert_eq!(
            AnthropicAdapter::new("k").timeout(),
            Duration::from_secs(600)
        );
        assert_eq!(
            AnthropicAdapter::with_base_url("k", "http://127.0.0.1:1/").timeout(),
            Duration::from_secs(600)
        );

        // A server that never answers: the call must end at ~timeout_secs.
        let server = serve_once(Reply::Hang);
        let adapter = AnthropicAdapter::from_profile_with(
            "slow-local",
            &server.base_url,
            None,
            Some(1),
            |_| panic!("no api_key_env: the environment must not be read"),
        )
        .unwrap();
        let started = Instant::now();
        let err = adapter.complete(&req()).unwrap_err().to_string();
        let elapsed = started.elapsed();
        drop(adapter);
        server.finish();
        assert!(
            err.starts_with("anthropic api: timed out after 1s"),
            "{err}"
        );
        assert!(err.contains("timeout_secs"), "{err}");
        assert!(elapsed < Duration::from_secs(8), "took {elapsed:?}");
    }

    #[test]
    fn transport_errors_are_reported_without_retry() {
        // Nothing listens on this port any more.
        let server = serve(vec![]);
        let base_url = server.base_url.clone();
        server.finish();
        let adapter = adapter_for(&base_url, Some("sk-ant-SECRET"));
        let err = adapter.complete(&req()).unwrap_err().to_string();
        assert!(err.starts_with("anthropic api: "), "{err}");
        assert!(!err.contains("after retry"), "{err}");
        assert!(!err.contains("SECRET"), "{err}");
    }

    #[test]
    fn loopback_url_detection() {
        for yes in [
            "http://127.0.0.1:11434",
            "http://127.0.0.1",
            "http://127.8.9.10:1/v1",
            "http://localhost:8080/",
            "https://LOCALHOST",
            "http://[::1]:11434/path",
            "http://user:pw@127.0.0.1:9?x=1",
        ] {
            assert!(is_loopback_url(yes), "{yes}");
        }
        for no in [
            "https://api.anthropic.com",
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example:80",
            "http://evil.example/127.0.0.1",
            "http://evil.example?@127.0.0.1",
            "http://10.0.0.1:11434",
            "http://[2001:db8::1]:80",
            "http://0.0.0.0:80",
            "",
        ] {
            assert!(!is_loopback_url(no), "{no}");
        }
    }

    #[test]
    fn loopback_endpoints_bypass_environment_proxies() {
        // A dead proxy in the environment (ALL_PROXY wins over the other
        // proxy variables): a loopback endpoint must still be reached
        // directly.
        let _env = EnvGuard::set(&[("ALL_PROXY", "http://127.0.0.1:9"), ("NO_PROXY", "")]);
        let server = serve_once(response(200, "OK", OK_BODY));
        let adapter = adapter_for(&server.base_url, None);
        assert_eq!(adapter.complete(&req()).unwrap().text, "hello");
        assert_eq!(server.finish().len(), 1);
    }

    #[test]
    fn debug_redacts_api_key() {
        let adapter = AnthropicAdapter::new("sk-ant-SECRET");
        let rendered = format!("{adapter:?}");
        assert!(!rendered.contains("SECRET"), "{rendered}");
        assert!(rendered.contains("<redacted>"));
        assert!(rendered.contains("600s"), "{rendered}");

        let keyless = adapter_for("http://127.0.0.1:1", None);
        let rendered = format!("{keyless:?}");
        assert!(rendered.contains("api_key: None"), "{rendered}");
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
        // The built-in profile must not be able to pick which secret
        // becomes the x-api-key header sent to api.anthropic.com.
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
        assert_eq!(adapter.api_key.as_deref(), Some("sk-ant-SECRET"));
        assert_eq!(adapter.base_url, ANTHROPIC_DEFAULT_BASE_URL);
        assert_eq!(adapter.timeout(), Duration::from_secs(DEFAULT_TIMEOUT_SECS));
        assert!(is_allowed_api_key_env("ANTHROPIC_API_KEY_staging2"));

        // Allowed but empty → the same hint as unset.
        let err = AnthropicAdapter::from_env_value("ANTHROPIC_API_KEY", |_| Some(String::new()))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "set ANTHROPIC_API_KEY to use the anthropic provider");
    }

    #[test]
    fn from_profile_allows_any_env_name_and_makes_the_key_optional() {
        // User-level profiles are user-owned: no ANTHROPIC_* restriction.
        let adapter = AnthropicAdapter::from_profile_with(
            "my-gateway",
            "https://gateway.example/anthropic///",
            Some("MY_GATEWAY_TOKEN"),
            Some(45),
            |name| {
                assert_eq!(name, "MY_GATEWAY_TOKEN");
                Some("tok-SECRET".into())
            },
        )
        .unwrap();
        assert_eq!(adapter.api_key.as_deref(), Some("tok-SECRET"));
        assert_eq!(adapter.base_url, "https://gateway.example/anthropic");
        assert_eq!(adapter.timeout(), Duration::from_secs(45));
        assert_eq!(adapter.name(), "anthropic");

        // No api_key_env → no key, the environment is never consulted, and
        // the timeout falls back to the default.
        let adapter = AnthropicAdapter::from_profile_with(
            "ollama-anthropic",
            "http://127.0.0.1:11434",
            None,
            None,
            |_| panic!("lookup must not run without api_key_env"),
        )
        .unwrap();
        assert_eq!(adapter.api_key, None);
        assert_eq!(adapter.timeout(), Duration::from_secs(DEFAULT_TIMEOUT_SECS));
    }

    #[test]
    fn from_profile_errors_name_the_variable_and_the_profile() {
        for value in [None, Some(String::new())] {
            let err = AnthropicAdapter::from_profile_with(
                "my-gateway",
                "https://gateway.example",
                Some("MY_GATEWAY_TOKEN"),
                None,
                |_| value.clone(),
            )
            .unwrap_err()
            .to_string();
            assert_eq!(err, "set MY_GATEWAY_TOKEN to use the my-gateway provider");
        }
        let err = AnthropicAdapter::from_profile_with(
            "my-gateway",
            "https://gateway.example",
            None,
            Some(0),
            |_| None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("my-gateway") && err.contains("timeout_secs"),
            "{err}"
        );

        // A value that is not shaped like a variable name — typically the
        // key itself, pasted into the wrong field — is refused unread and
        // is NOT echoed.
        for pasted in ["sk-ant-api03-SECRET", "", "A=B", "MY KEY", "1ST", "K\n"] {
            let err = AnthropicAdapter::from_profile_with(
                "my-gateway",
                "https://gateway.example",
                Some(pasted),
                None,
                |_| panic!("lookup must not run for a refused name"),
            )
            .unwrap_err()
            .to_string();
            assert!(
                err.contains("my-gateway") && err.contains("api_key_env"),
                "{err}"
            );
            assert!(!err.contains("SECRET") && !err.contains('\n'), "{err}");
        }
        assert!(is_env_var_name("MY_GATEWAY_TOKEN") && is_env_var_name("_k9"));

        // The public entry point reads the real environment.
        let err = AnthropicAdapter::from_profile(
            "my-gateway",
            "https://gateway.example",
            Some("RUHARNESS_TEST_UNSET_PROFILE_KEY_VAR"),
            None,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            err,
            "set RUHARNESS_TEST_UNSET_PROFILE_KEY_VAR to use the my-gateway provider"
        );
    }
}
