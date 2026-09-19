//! Provider profiles (docs/SCHEMAS.md "M3 additions: executor + provider
//! profiles"): turn the profile NAME a target's `harness.toml` selects into
//! a ready [`ProviderAdapter`].
//!
//! Trust boundary: target-owned files are hostile input, so a target may
//! only *name* a profile (and a model string, which adapters place in the
//! request body only). Endpoints and credentials live in USER-level config —
//! the file at `$RUHARNESS_PROVIDERS`, else
//! `~/.config/ruharness/providers.toml` ([`profiles_path`]) — and never come
//! from the target. Consequences enforced here:
//!
//! - the requested name is shape-checked before it is echoed or looked up;
//! - the profiles path must be absolute (a relative `$RUHARNESS_PROVIDERS`
//!   would resolve against the working directory, which may BE the target);
//! - the built-in names `external`, `replay`, `anthropic` are reserved: a
//!   user table of the same name is an error, never a silent override in
//!   either direction;
//! - profile tables reject unknown keys, so a typo such as `api_key_evn`
//!   cannot silently turn authentication off.
//!
//! ```toml
//! [providers.ollama-anthropic]
//! kind = "anthropic"                  # see `supported kinds` in errors
//! base_url = "http://127.0.0.1:11434" # required; http:// or https://
//! # api_key_env = "MY_KEY"            # optional; omitted = no auth header
//! context_tokens = 32768              # optional
//! timeout_secs = 600                  # optional (default 600)
//! ```
//!
//! Adding a provider kind is one new adapter file plus one arm in the
//! private `kind_builder` match; everything else (the supported-kinds list
//! in errors, validation, resolution) derives from that match.

use crate::adapters::{is_env_var_name, AnthropicAdapter, TraceAdapter};
use harness_core::error::Error;
use harness_core::traits::{CompletionRequest, CompletionResponse, ProviderAdapter};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Environment variable overriding the profiles file location.
pub const PROFILES_ENV: &str = "RUHARNESS_PROVIDERS";

/// Profile names that need no file and can never be redefined by one.
const BUILTIN_PROFILES: [&str; 3] = ["external", "replay", "anthropic"];

/// The built-in `anthropic` profile's key variable.
const BUILTIN_ANTHROPIC_KEY_ENV: &str = "ANTHROPIC_API_KEY";

/// Every `kind` the schema defines (docs/SCHEMAS.md). Which of them THIS
/// build supports is derived from [`kind_builder`], never listed twice.
const SCHEMA_KINDS: [&str; 2] = ["anthropic", "openai-compat"];

/// Closed value set of `max_tokens_field` (docs/SCHEMAS.md).
const MAX_TOKENS_FIELDS: [&str; 2] = ["max_tokens", "max_completion_tokens"];

/// Longest profile name accepted.
const PROFILE_NAME_MAX_LEN: usize = 64;

/// Longest prefix of an offending value echoed in an error, in chars.
const ERROR_ECHO_MAX_CHARS: usize = 48;

/// Environment lookup, injected so resolution is testable without mutating
/// the process environment.
pub(crate) type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// Constructor of one provider kind's adapter from a validated profile.
pub(crate) type KindBuilder =
    fn(&ProviderProfile, EnvLookup<'_>) -> Result<Box<dyn ProviderAdapter>, Error>;

/// A provider profile resolved to a ready adapter.
pub struct ResolvedProvider {
    /// The adapter to route completions through.
    pub adapter: Box<dyn ProviderAdapter>,
    /// The profile name that was resolved (recorded as an attempt's
    /// `provider`).
    pub profile: String,
    /// The profile's kind: `external`, `replay`, or a wire-protocol kind
    /// such as `anthropic` (recorded as an attempt's `provider_kind`).
    pub kind: String,
    /// The endpoint's context window in tokens, when the profile declares
    /// one — enables the truncation preflight (docs/SCHEMAS.md).
    pub context_tokens: Option<u32>,
    /// True when calls reach a live model (network); false for the
    /// trace-backed `external` and `replay` profiles.
    pub live: bool,
}

/// Message prefix of the context preflight refusal ([`checked_complete`]).
pub const PROMPT_DOES_NOT_FIT: &str = "prompt does not fit provider context";
/// Message prefix of the after-call truncation error ([`checked_complete`]).
pub const PROMPT_TRUNCATED: &str = "prompt truncated by server";

/// Prompt size in bytes (system + user).
pub(crate) fn prompt_bytes(request: &CompletionRequest) -> u64 {
    (request.system.len() + request.user.len()) as u64
}

/// Context preflight (docs/SCHEMAS.md "Provider profiles"): refuse a request
/// that cannot fit the profile's declared context window —
/// `prompt_bytes/3 + max_tokens > context_tokens`. A profile that declares
/// no window passes.
pub(crate) fn preflight(
    provider: &ResolvedProvider,
    request: &CompletionRequest,
) -> Result<(), Error> {
    let Some(context) = provider.context_tokens else {
        return Ok(());
    };
    let bytes = prompt_bytes(request);
    let needed = bytes / 3 + u64::from(request.max_tokens);
    if needed > u64::from(context) {
        return Err(Error::Invariant(format!(
            "{PROMPT_DOES_NOT_FIT}: {bytes} prompt bytes / 3 + max_tokens {} = {needed} tokens > \
             context_tokens {context} of profile `{}` — use a profile with a larger window or \
             lower max_tokens",
            request.max_tokens, provider.profile
        )));
    }
    Ok(())
}

/// True when `e` is one of [`checked_complete`]'s own refusals (the context
/// preflight or the truncation check) rather than an adapter error.
pub(crate) fn is_context_error(e: &Error) -> bool {
    matches!(e, Error::Invariant(m)
        if m.starts_with(PROMPT_DOES_NOT_FIT) || m.starts_with(PROMPT_TRUNCATED))
}

/// THE way a completion is requested — by the triage pass and the executor
/// alike — so both get the same two context guards (docs/SCHEMAS.md
/// "Provider profiles"):
///
/// 1. **Before the call**, when the profile declares `context_tokens`: a
///    request with `prompt_bytes/3 + max_tokens > context_tokens` is refused
///    ([`PROMPT_DOES_NOT_FIT`]); nothing is sent.
/// 2. **After the call**, when the response reports `input_tokens > 0` but
///    fewer than `prompt_bytes/6`: the endpoint silently dropped part of the
///    prompt, so whatever it answered is not an answer to this request
///    ([`PROMPT_TRUNCATED`]). This applies to EVERY provider: a trace-backed
///    response that carries real token counts is a recording of a live call
///    and is just as void. `0` means "not reported" and is never checked.
///
/// Both are HARNESS errors, never model outcomes. This function records
/// nothing: callers record a trace only after it returned `Ok`, so a
/// refused or truncated call can never leave a normal replayable trace
/// behind. Adapter errors (including the `external` hand-off's "awaiting
/// response") are propagated unchanged.
pub fn checked_complete(
    provider: &ResolvedProvider,
    req: &CompletionRequest,
) -> Result<CompletionResponse, Error> {
    preflight(provider, req)?;
    let response = provider.adapter.complete(req)?;
    let sent = prompt_bytes(req);
    if response.input_tokens > 0 && response.input_tokens < sent / 6 {
        return Err(Error::Invariant(format!(
            "{PROMPT_TRUNCATED}: provider `{}` reported {} input tokens for a {sent}-byte prompt \
             (at least {} expected) — the endpoint's context window is smaller than the prompt; \
             raise it and declare context_tokens in the profile",
            provider.profile,
            response.input_tokens,
            sent / 6
        )));
    }
    Ok(response)
}

impl std::fmt::Debug for ResolvedProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedProvider")
            .field("adapter", &self.adapter.name())
            .field("profile", &self.profile)
            .field("kind", &self.kind)
            .field("context_tokens", &self.context_tokens)
            .field("live", &self.live)
            .finish()
    }
}

/// One validated `[providers.<name>]` table — what a kind's adapter
/// constructor receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProfile {
    /// The table name (`<name>` in `[providers.<name>]`).
    pub name: String,
    /// Wire-protocol kind; always one this build supports.
    pub kind: String,
    /// Endpoint base URL (`http://` or `https://`).
    pub base_url: String,
    /// Environment variable holding the API key; `None` = no auth header.
    /// Any name is allowed: the profiles file is user-owned.
    pub api_key_env: Option<String>,
    /// Context window in tokens, when declared (always > 0).
    pub context_tokens: Option<u32>,
    /// End-to-end timeout of one call in seconds, when declared (always
    /// > 0); `None` = the adapter's default (600).
    pub timeout_secs: Option<u64>,
    /// `max_tokens` | `max_completion_tokens`: which request field carries
    /// the token budget. Parsed and carried for the kinds that need it;
    /// ignored by `anthropic`.
    pub max_tokens_field: Option<String>,
}

/// The kind dispatch. Adding a provider kind = one new adapter file plus
/// ONE ARM HERE, e.g. `"openai-compat" => Some(crate::openai_compat::build)`.
fn kind_builder(kind: &str) -> Option<KindBuilder> {
    match kind {
        "anthropic" => Some(build_anthropic),
        "openai-compat" => Some(crate::openai_compat::build),
        _ => None,
    }
}

/// The schema kinds this build can construct, in schema order.
fn supported_kinds() -> Vec<&'static str> {
    SCHEMA_KINDS
        .iter()
        .copied()
        .filter(|kind| kind_builder(kind).is_some())
        .collect()
}

/// The `anthropic` kind: [`AnthropicAdapter`] from a user-level profile.
fn anthropic_from_profile(
    profile: &ProviderProfile,
    env: EnvLookup<'_>,
) -> Result<AnthropicAdapter, Error> {
    AnthropicAdapter::from_profile_with(
        &profile.name,
        &profile.base_url,
        profile.api_key_env.as_deref(),
        profile.timeout_secs,
        |name| env_string(env, name),
    )
}

/// [`KindBuilder`] for `anthropic`.
fn build_anthropic(
    profile: &ProviderProfile,
    env: EnvLookup<'_>,
) -> Result<Box<dyn ProviderAdapter>, Error> {
    Ok(Box::new(anthropic_from_profile(profile, env)?))
}

/// A variable's value as text; a non-UTF-8 value counts as unset (an API
/// key must be header-safe text anyway).
fn env_string(env: EnvLookup<'_>, name: &str) -> Option<String> {
    env(name).and_then(|value| value.into_string().ok())
}

/// Debug-escaped (single-line) rendering of an untrusted value for error
/// messages, truncated to [`ERROR_ECHO_MAX_CHARS`] chars.
fn short_debug(value: &str) -> String {
    let mut shown: String = value.chars().take(ERROR_ECHO_MAX_CHARS).collect();
    if shown.len() < value.len() {
        shown.push('…');
    }
    format!("{shown:?}")
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]*$`, at most [`PROFILE_NAME_MAX_LEN`] bytes.
fn is_profile_name(name: &str) -> bool {
    name.len() <= PROFILE_NAME_MAX_LEN
        && name
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Where user-level provider profiles are read from: `$RUHARNESS_PROVIDERS`
/// when set (and non-empty), else `~/.config/ruharness/providers.toml`
/// (`$HOME`, falling back to `%USERPROFILE%`). `None` when neither variable
/// is available. The file need not exist.
pub fn profiles_path() -> Option<PathBuf> {
    profiles_path_with(&|name| std::env::var_os(name))
}

/// [`profiles_path`] with the environment lookup injected.
fn profiles_path_with(env: EnvLookup<'_>) -> Option<PathBuf> {
    let non_empty = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(explicit) = non_empty(PROFILES_ENV) {
        return Some(PathBuf::from(explicit));
    }
    let home = non_empty("HOME").or_else(|| non_empty("USERPROFILE"))?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("ruharness")
            .join("providers.toml"),
    )
}

/// The profiles file as parsed: tables only, semantics unchecked.
#[derive(serde::Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    providers: BTreeMap<String, RawProfile>,
}

/// One `[providers.<name>]` table before validation. Every key is optional
/// at this layer so that a mistake in one profile never breaks resolving
/// another; unknown keys are refused (a typo must not silently drop
/// `api_key_env`).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    kind: Option<String>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    context_tokens: Option<u32>,
    timeout_secs: Option<u64>,
    max_tokens_field: Option<String>,
}

/// A toml parse error as `line N: <message>`. toml's default rendering
/// quotes the offending source line; this file sits next to credentials
/// (and is where a key gets pasted by mistake, e.g. `api_key = "sk-…"`), so
/// file content is never echoed into an error.
fn toml_error(path: &Path, text: &str, e: &toml::de::Error) -> Error {
    let message = match e.span() {
        Some(span) => {
            let upto = span.start.min(text.len());
            let line = text.as_bytes()[..upto]
                .iter()
                .filter(|b| **b == b'\n')
                .count()
                + 1;
            format!("line {line}: {}", e.message())
        }
        None => e.message().to_string(),
    };
    Error::parse(path, message)
}

/// Parse profiles-file text and enforce the reserved built-in names.
fn parse_profiles(path: &Path, text: &str) -> Result<BTreeMap<String, RawProfile>, Error> {
    let file: ProfilesFile = toml::from_str(text).map_err(|e| toml_error(path, text, &e))?;
    if let Some(shadowed) = BUILTIN_PROFILES
        .iter()
        .find(|builtin| file.providers.contains_key(**builtin))
    {
        return Err(Error::parse(
            path,
            format!(
                "[providers.{shadowed}] shadows the built-in `{shadowed}` profile: the names {} \
                 are reserved — rename the profile",
                BUILTIN_PROFILES.join(", ")
            ),
        ));
    }
    Ok(file.providers)
}

/// Load the profiles file. `Ok(None)` when it does not exist (built-ins
/// need no file); any other failure is an error.
fn load_profiles(path: &Path) -> Result<Option<BTreeMap<String, RawProfile>>, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_profiles(path, &text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Validate one raw table into a [`ProviderProfile`]. Errors name the file
/// and the table.
fn validate_profile(path: &Path, name: &str, raw: RawProfile) -> Result<ProviderProfile, Error> {
    let bad = |what: String| Error::parse(path, format!("[providers.{name}] {what}"));
    let supported = supported_kinds().join(", ");

    let kind = raw.kind.ok_or_else(|| {
        bad(format!(
            "missing required key `kind` (supported kinds: {supported})"
        ))
    })?;
    if kind_builder(&kind).is_none() {
        return Err(bad(format!(
            "unsupported kind {} (supported kinds: {supported})",
            short_debug(&kind)
        )));
    }

    let base_url = raw
        .base_url
        .ok_or_else(|| bad("missing required key `base_url`".into()))?;
    let has_host = base_url
        .strip_prefix("http://")
        .or_else(|| base_url.strip_prefix("https://"))
        .is_some_and(|rest| !rest.is_empty());
    let is_clean = !base_url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control());
    if !(has_host && is_clean) {
        return Err(bad(format!(
            "base_url {} must start with http:// or https:// and contain no whitespace",
            short_debug(&base_url)
        )));
    }

    if raw
        .api_key_env
        .as_deref()
        .is_some_and(|env_name| !is_env_var_name(env_name))
    {
        // Deliberately not echoed: the classic mistake is pasting the key
        // itself into this field, and errors end up in logs.
        return Err(bad(
            "api_key_env is not an environment variable name (expected \
             ^[A-Za-z_][A-Za-z0-9_]*$; value not shown) — it names the variable that holds the \
             key, never the key itself; omit it to send no auth header"
                .into(),
        ));
    }
    if raw.context_tokens == Some(0) {
        return Err(bad("context_tokens must be greater than 0".into()));
    }
    if raw.timeout_secs == Some(0) {
        return Err(bad("timeout_secs must be greater than 0".into()));
    }
    if let Some(field) = &raw.max_tokens_field {
        if !MAX_TOKENS_FIELDS.contains(&field.as_str()) {
            return Err(bad(format!(
                "max_tokens_field {} is not one of: {}",
                short_debug(field),
                MAX_TOKENS_FIELDS.join(", ")
            )));
        }
    }

    Ok(ProviderProfile {
        name: name.to_string(),
        kind,
        base_url,
        api_key_env: raw.api_key_env,
        context_tokens: raw.context_tokens,
        timeout_secs: raw.timeout_secs,
        max_tokens_field: raw.max_tokens_field,
    })
}

/// Resolve a provider profile name to a ready adapter.
///
/// Built-ins need no file: `external` and `replay` are [`TraceAdapter`]s
/// over `traces_dir` (kind = name, not live); `anthropic` is the live
/// Messages API at `https://api.anthropic.com` keyed by `ANTHROPIC_API_KEY`
/// (unset → `set ANTHROPIC_API_KEY to use the anthropic provider`). Any
/// other name must be a `[providers.<name>]` table in the user-level
/// profiles file ([`profiles_path`]); an unknown name is an error naming
/// the file that was searched.
///
/// When the profiles file exists it is ALWAYS parsed — also for a built-in
/// name — so that a table shadowing a built-in is reported instead of
/// silently ignored. Only the requested table is validated semantically.
pub fn resolve(profile: &str, traces_dir: &Path) -> Result<ResolvedProvider, Error> {
    resolve_with(profile, traces_dir, &|name| std::env::var_os(name))
}

/// [`resolve`] with the environment lookup injected.
fn resolve_with(
    profile: &str,
    traces_dir: &Path,
    env: EnvLookup<'_>,
) -> Result<ResolvedProvider, Error> {
    if !is_profile_name(profile) {
        return Err(Error::Invariant(format!(
            "invalid provider profile name {}: expected ^[A-Za-z0-9][A-Za-z0-9._-]*$ (at most \
             {PROFILE_NAME_MAX_LEN} characters)",
            short_debug(profile)
        )));
    }

    let path = profiles_path_with(env);
    if let Some(relative) = path.as_deref().filter(|p| !p.is_absolute()) {
        return Err(Error::Invariant(format!(
            "provider profiles path {} is not absolute: set {PROFILES_ENV} to an absolute path \
             (a relative one would resolve inside whatever directory the harness runs in, which \
             may be an untrusted target)",
            relative.display()
        )));
    }
    let mut profiles = match &path {
        Some(path) => load_profiles(path)?,
        None => None,
    };

    let resolved =
        |adapter: Box<dyn ProviderAdapter>, kind: &str, context_tokens, live| ResolvedProvider {
            adapter,
            profile: profile.to_string(),
            kind: kind.to_string(),
            context_tokens,
            live,
        };
    match profile {
        "external" | "replay" => {
            let adapter = TraceAdapter::new(traces_dir, profile == "external");
            return Ok(resolved(Box::new(adapter), profile, None, false));
        }
        "anthropic" => {
            let adapter = AnthropicAdapter::from_env_value(BUILTIN_ANTHROPIC_KEY_ENV, |name| {
                env_string(env, name)
            })?;
            return Ok(resolved(Box::new(adapter), "anthropic", None, true));
        }
        _ => {}
    }

    let raw = profiles.as_mut().and_then(|tables| tables.remove(profile));
    let (Some(path), Some(raw)) = (&path, raw) else {
        return Err(unknown_profile(profile, path.as_deref(), profiles.as_ref()));
    };
    let validated = validate_profile(path, profile, raw)?;
    let build = kind_builder(&validated.kind).ok_or_else(|| {
        Error::Invariant(format!(
            "internal: validated kind `{}` has no builder",
            validated.kind
        ))
    })?;
    let adapter = build(&validated, env)?;
    Ok(resolved(
        adapter,
        &validated.kind,
        validated.context_tokens,
        true,
    ))
}

/// The unknown-profile error: names the built-ins and the file searched.
fn unknown_profile(
    profile: &str,
    path: Option<&Path>,
    profiles: Option<&BTreeMap<String, RawProfile>>,
) -> Error {
    let builtins = BUILTIN_PROFILES.join(", ");
    let searched = match (path, profiles) {
        (None, _) => format!(
            "no profiles file could be located (set {PROFILES_ENV}, or HOME for \
             ~/.config/ruharness/providers.toml)"
        ),
        (Some(path), None) => format!("the profiles file {} does not exist", path.display()),
        (Some(path), Some(tables)) => {
            let defined: Vec<&str> = tables
                .keys()
                .map(String::as_str)
                .filter(|name| is_profile_name(name))
                .collect();
            format!(
                "there is no [providers.{profile}] table in {} (it defines: {})",
                path.display(),
                if defined.is_empty() {
                    "no profiles".to_string()
                } else {
                    defined.join(", ")
                }
            )
        }
    };
    Error::Invariant(format!(
        "unknown provider profile `{profile}`: not a built-in ({builtins}) and {searched}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::mock_http::{response, serve_once};
    use crate::adapters::test_env::EnvGuard;
    use std::time::Duration;

    const OK_BODY: &str = r#"{"content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":2,"output_tokens":1},"stop_reason":"end_turn"}"#;

    const EXAMPLE: &str = r#"
[providers.ollama-anthropic]
kind = "anthropic"
base_url = "http://127.0.0.1:11434"
context_tokens = 32768
timeout_secs = 45

[providers.gateway]
kind = "anthropic"
base_url = "https://gateway.example/anthropic/"
api_key_env = "MY_GATEWAY_TOKEN"
max_tokens_field = "max_completion_tokens"

[providers.later]
kind = "openai-compat"
base_url = "http://127.0.0.1:8080/v1"
max_tokens_field = "max_tokens"
"#;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "harness-llm-providers-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write `text` as a profiles file in a fresh temp dir.
    fn profiles_file(name: &str, text: &str) -> PathBuf {
        let path = temp_dir(name).join("providers.toml");
        std::fs::write(&path, text).unwrap();
        path
    }

    /// An injected environment: exactly these variables exist.
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: BTreeMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |name| vars.get(name).cloned()
    }

    fn resolve_in(
        profile: &str,
        file: &Path,
        extra: &[(&str, &str)],
    ) -> Result<ResolvedProvider, Error> {
        let mut pairs = vec![(PROFILES_ENV, file.to_str().unwrap())];
        pairs.extend_from_slice(extra);
        resolve_with(profile, Path::new("/nonexistent/traces"), &env_of(&pairs))
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            model: "llama3.2-1b-32k".into(),
            system: "s".into(),
            user: "u".into(),
            max_tokens: 64,
        }
    }

    // ---- built-ins -----------------------------------------------------

    #[test]
    fn builtin_trace_profiles_need_no_file() {
        let traces = temp_dir("builtin-traces");
        // No profiles file anywhere: not even a locatable path.
        for (name, awaiting) in [("external", true), ("replay", false)] {
            let got = resolve_with(name, &traces, &env_of(&[])).unwrap();
            assert_eq!((got.profile.as_str(), got.kind.as_str()), (name, name));
            assert_eq!(got.adapter.name(), name);
            assert!(!got.live);
            assert_eq!(got.context_tokens, None);
            // The adapter really is a TraceAdapter over `traces_dir`.
            let err = got.adapter.complete(&req()).unwrap_err().to_string();
            assert_eq!(err.starts_with("awaiting response: "), awaiting, "{err}");
            assert!(err.contains(traces.to_str().unwrap()), "{err}");
        }
        // A path whose file does not exist is fine too.
        let missing = temp_dir("builtin-missing").join("providers.toml");
        assert!(!resolve_in("external", &missing, &[]).unwrap().live);
    }

    #[test]
    fn builtin_anthropic_reads_anthropic_api_key() {
        let got = resolve_with(
            "anthropic",
            Path::new("/unused"),
            &env_of(&[("ANTHROPIC_API_KEY", "sk-ant-SECRET")]),
        )
        .unwrap();
        assert_eq!(got.profile, "anthropic");
        assert_eq!(got.kind, "anthropic");
        assert_eq!(got.adapter.name(), "anthropic");
        assert!(got.live);
        assert_eq!(got.context_tokens, None);
        let rendered = format!("{got:?}");
        assert!(!rendered.contains("SECRET"), "{rendered}");

        for env in [env_of(&[]), env_of(&[("ANTHROPIC_API_KEY", "")])] {
            let err = resolve_with("anthropic", Path::new("/unused"), &env)
                .unwrap_err()
                .to_string();
            assert_eq!(err, "set ANTHROPIC_API_KEY to use the anthropic provider");
        }
    }

    #[test]
    fn builtin_anthropic_targets_the_default_endpoint_with_default_timeout() {
        let adapter =
            AnthropicAdapter::from_env_value(BUILTIN_ANTHROPIC_KEY_ENV, |_| Some("k".into()))
                .unwrap();
        let rendered = format!("{adapter:?}");
        assert!(rendered.contains("https://api.anthropic.com"), "{rendered}");
        assert_eq!(adapter.timeout(), Duration::from_secs(600));
    }

    // ---- user profiles -------------------------------------------------

    #[test]
    fn user_profile_fields_are_parsed_and_carried() {
        let path = profiles_file("fields", EXAMPLE);
        let mut tables = load_profiles(&path).unwrap().unwrap();

        let ollama = validate_profile(
            &path,
            "ollama-anthropic",
            tables.remove("ollama-anthropic").unwrap(),
        )
        .unwrap();
        assert_eq!(
            ollama,
            ProviderProfile {
                name: "ollama-anthropic".into(),
                kind: "anthropic".into(),
                base_url: "http://127.0.0.1:11434".into(),
                api_key_env: None,
                context_tokens: Some(32768),
                timeout_secs: Some(45),
                max_tokens_field: None,
            }
        );
        // The profile's timeout reaches the adapter; no key → no env read.
        let adapter = anthropic_from_profile(&ollama, &|name| {
            panic!("environment must not be read without api_key_env (asked for {name})")
        })
        .unwrap();
        assert_eq!(adapter.timeout(), Duration::from_secs(45));

        // max_tokens_field is parsed and carried, and ignored by anthropic.
        let gateway =
            validate_profile(&path, "gateway", tables.remove("gateway").unwrap()).unwrap();
        assert_eq!(gateway.api_key_env.as_deref(), Some("MY_GATEWAY_TOKEN"));
        assert_eq!(
            gateway.max_tokens_field.as_deref(),
            Some("max_completion_tokens")
        );
        assert_eq!(gateway.timeout_secs, None);
        let adapter =
            anthropic_from_profile(&gateway, &env_of(&[("MY_GATEWAY_TOKEN", "tok")])).unwrap();
        assert_eq!(
            adapter.timeout(),
            Duration::from_secs(600),
            "default timeout"
        );
    }

    #[test]
    fn user_profile_resolves_to_a_live_anthropic_adapter() {
        let path = profiles_file("resolve", EXAMPLE);
        let got = resolve_in("ollama-anthropic", &path, &[]).unwrap();
        assert_eq!(got.profile, "ollama-anthropic");
        assert_eq!(got.kind, "anthropic");
        assert_eq!(got.adapter.name(), "anthropic");
        assert_eq!(got.context_tokens, Some(32768));
        assert!(got.live);

        let got = resolve_in("gateway", &path, &[("MY_GATEWAY_TOKEN", "tok-SECRET")]).unwrap();
        assert_eq!(
            (got.kind.as_str(), got.context_tokens, got.live),
            ("anthropic", None, true)
        );
        assert!(!format!("{got:?}").contains("SECRET"));
    }

    #[test]
    fn user_profile_key_env_may_have_any_name_but_must_be_set() {
        let path = profiles_file("key-env", EXAMPLE);
        for extra in [&[][..], &[("MY_GATEWAY_TOKEN", "")][..]] {
            let err = resolve_in("gateway", &path, extra).unwrap_err().to_string();
            assert_eq!(err, "set MY_GATEWAY_TOKEN to use the gateway provider");
        }
        // The built-in's ANTHROPIC_* restriction does not apply: the file is
        // user-owned, so `MY_GATEWAY_TOKEN` is accepted as-is.
        resolve_in("gateway", &path, &[("MY_GATEWAY_TOKEN", "tok")]).unwrap();
    }

    #[test]
    fn user_profile_wire_behavior_end_to_end() {
        // With api_key_env: the variable's value is the x-api-key header,
        // and the profile's base_url is where the request goes.
        let server = serve_once(response(200, "OK", OK_BODY));
        let path = profiles_file(
            "wire-key",
            &format!(
                "[providers.local]\nkind = \"anthropic\"\nbase_url = \"{}\"\n\
                 api_key_env = \"LOCAL_TOKEN\"\ntimeout_secs = 30\n",
                server.base_url
            ),
        );
        let got = resolve_in("local", &path, &[("LOCAL_TOKEN", "tok-123")]).unwrap();
        let resp = got.adapter.complete(&req()).unwrap();
        assert_eq!(resp.text, "hi");
        let seen = server.finish();
        assert_eq!(seen[0].request_line, "POST /v1/messages HTTP/1.1");
        assert_eq!(seen[0].header("x-api-key"), Some("tok-123"));
        let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
        assert_eq!(body["model"], "llama3.2-1b-32k");

        // Without api_key_env: no auth header at all, even when the
        // built-in's variable happens to be set.
        let server = serve_once(response(200, "OK", OK_BODY));
        let path = profiles_file(
            "wire-nokey",
            &format!(
                "[providers.local]\nkind = \"anthropic\"\nbase_url = \"{}\"\n",
                server.base_url
            ),
        );
        let got = resolve_in("local", &path, &[("ANTHROPIC_API_KEY", "sk-ant-SECRET")]).unwrap();
        got.adapter.complete(&req()).unwrap();
        let seen = server.finish();
        assert_eq!(seen[0].header("x-api-key"), None);
        assert_eq!(seen[0].header("authorization"), None);
    }

    #[test]
    fn unknown_kind_lists_the_supported_kinds() {
        assert_eq!(supported_kinds(), ["anthropic", "openai-compat"]);
        let path = profiles_file(
            "kinds",
            &format!("{EXAMPLE}\n[providers.odd]\nkind = \"gemini\"\nbase_url = \"http://x\"\n"),
        );
        let err = resolve_in("odd", &path, &[]).unwrap_err().to_string();
        assert!(err.contains("[providers.odd]"), "{err}");
        assert!(err.contains("unsupported kind \"gemini\""), "{err}");
        assert!(
            err.contains("(supported kinds: anthropic, openai-compat)"),
            "{err}"
        );
        assert!(err.contains(path.to_str().unwrap()), "{err}");
        // A broken sibling never blocks a healthy profile.
        resolve_in("ollama-anthropic", &path, &[]).unwrap();
        assert_eq!(
            resolve_in("later", &path, &[]).unwrap().kind,
            "openai-compat"
        );
    }

    #[test]
    fn every_supported_kind_is_a_schema_kind_with_a_builder() {
        for kind in supported_kinds() {
            assert!(SCHEMA_KINDS.contains(&kind));
            assert!(kind_builder(kind).is_some());
        }
        assert!(kind_builder("external").is_none());
        assert!(kind_builder("").is_none());
    }

    #[test]
    fn profile_validation_errors() {
        let case = |label: &str, table: &str| {
            let path = profiles_file(&format!("invalid-{label}"), table);
            let err = resolve_in("p", &path, &[]).unwrap_err().to_string();
            assert!(err.contains(path.to_str().unwrap()), "{label}: {err}");
            assert!(!err.contains('\n'), "{label}: {err}");
            err
        };
        let err = case("no-kind", "[providers.p]\nbase_url = \"http://x\"\n");
        assert!(err.contains("missing required key `kind`"), "{err}");
        assert!(err.contains("supported kinds: anthropic"), "{err}");

        let err = case("no-url", "[providers.p]\nkind = \"anthropic\"\n");
        assert!(
            err.contains("[providers.p] missing required key `base_url`"),
            "{err}"
        );

        for (label, url) in [
            ("url-scheme", "ftp://x"),
            ("url-bare", "127.0.0.1:11434"),
            ("url-file", "file:///etc/passwd"),
            ("url-empty-host", "https://"),
            ("url-upper", "HTTP://x"),
            ("url-space", "http://x y"),
            ("url-newline", "http://x\\nHost: evil"),
        ] {
            let err = case(
                label,
                &format!("[providers.p]\nkind = \"anthropic\"\nbase_url = \"{url}\"\n"),
            );
            assert!(
                err.contains("must start with http:// or https://"),
                "{label}: {err}"
            );
        }

        let base = "[providers.p]\nkind = \"anthropic\"\nbase_url = \"http://x\"\n";
        let err = case("timeout-zero", &format!("{base}timeout_secs = 0\n"));
        assert!(err.contains("timeout_secs must be greater than 0"), "{err}");
        let err = case("context-zero", &format!("{base}context_tokens = 0\n"));
        assert!(
            err.contains("context_tokens must be greater than 0"),
            "{err}"
        );
        let err = case(
            "field",
            &format!("{base}max_tokens_field = \"n_predict\"\n"),
        );
        assert!(err.contains("max_tokens_field \"n_predict\""), "{err}");
        assert!(err.contains("max_tokens, max_completion_tokens"), "{err}");
        // A pasted key (or any non-name) in api_key_env is refused and
        // never echoed.
        for (label, name) in [
            ("env-empty", ""),
            ("env-eq", "A=B"),
            ("env-space", "A B"),
            ("env-pasted", "sk-ant-api03-SECRET"),
        ] {
            let err = case(label, &format!("{base}api_key_env = \"{name}\"\n"));
            assert!(
                err.contains("[providers.p] api_key_env is not an environment variable name"),
                "{label}: {err}"
            );
            assert!(!err.contains("SECRET"), "{label}: {err}");
        }

        // Type errors and unknown keys are parse errors naming the file
        // and the line — without quoting file content (a key pasted into a
        // made-up `api_key` field must not reach the error).
        let err = case("typo", &format!("{base}api_key_evn = \"MY_KEY\"\n"));
        assert!(err.starts_with("parse error in "), "{err}");
        assert!(err.contains("line 4: unknown field `api_key_evn`"), "{err}");
        let err = case("pasted", &format!("{base}api_key = \"sk-ant-SECRET\"\n"));
        assert!(err.contains("unknown field `api_key`"), "{err}");
        assert!(!err.contains("SECRET"), "{err}");
        let err = case("type", &format!("{base}context_tokens = \"big\"\n"));
        assert!(err.starts_with("parse error in "), "{err}");
        assert!(err.contains("line 4: "), "{err}");
        let err = case("negative", &format!("{base}context_tokens = -1\n"));
        assert!(err.starts_with("parse error in "), "{err}");
    }

    #[test]
    fn user_profiles_may_not_shadow_builtins() {
        for builtin in BUILTIN_PROFILES {
            let path = profiles_file(
                &format!("shadow-{builtin}"),
                &format!(
                    "{EXAMPLE}\n[providers.{builtin}]\nkind = \"anthropic\"\n\
                     base_url = \"http://127.0.0.1:1\"\n"
                ),
            );
            // Refused whichever profile is being resolved: the shadowed
            // built-in itself, another built-in, or a healthy user profile.
            for requested in [builtin, "external", "ollama-anthropic"] {
                let err = resolve_in(requested, &path, &[("ANTHROPIC_API_KEY", "k")])
                    .unwrap_err()
                    .to_string();
                assert!(
                    err.contains(&format!("[providers.{builtin}] shadows the built-in")),
                    "{requested}: {err}"
                );
                assert!(err.contains("external, replay, anthropic"), "{err}");
                assert!(err.contains(path.to_str().unwrap()), "{err}");
            }
        }
    }

    #[test]
    fn unknown_profile_names_the_file_searched() {
        // File exists: names it and what it defines.
        let path = profiles_file("unknown", EXAMPLE);
        let err = resolve_in("nope", &path, &[]).unwrap_err().to_string();
        assert!(err.starts_with("unknown provider profile `nope`"), "{err}");
        assert!(err.contains("external, replay, anthropic"), "{err}");
        assert!(err.contains(path.to_str().unwrap()), "{err}");
        assert!(err.contains("gateway, later, ollama-anthropic"), "{err}");

        // File exists but defines nothing.
        let empty = profiles_file("unknown-empty", "# nothing yet\n");
        let err = resolve_in("nope", &empty, &[]).unwrap_err().to_string();
        assert!(err.contains(empty.to_str().unwrap()), "{err}");
        assert!(err.contains("it defines: no profiles"), "{err}");

        // File missing: still names the path that was searched.
        let missing = temp_dir("unknown-missing").join("providers.toml");
        let err = resolve_in("nope", &missing, &[]).unwrap_err().to_string();
        assert!(err.contains(missing.to_str().unwrap()), "{err}");
        assert!(err.contains("does not exist"), "{err}");

        // Default location derived from HOME.
        let home = temp_dir("unknown-home");
        let err = resolve_with(
            "nope",
            Path::new("/unused"),
            &env_of(&[("HOME", home.to_str().unwrap())]),
        )
        .unwrap_err()
        .to_string();
        let default = home.join(".config/ruharness/providers.toml");
        assert!(err.contains(default.to_str().unwrap()), "{err}");

        // No location at all.
        let err = resolve_with("nope", Path::new("/unused"), &env_of(&[]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("no profiles file could be located"), "{err}");
        assert!(err.contains(PROFILES_ENV), "{err}");
    }

    #[test]
    fn hostile_profile_names_are_refused_before_any_lookup() {
        // harness.toml (hostile) chooses the name: it must be a clean token.
        let long = "a".repeat(PROFILE_NAME_MAX_LEN + 1);
        for hostile in [
            "",
            "../providers",
            "a/b",
            "a b",
            "-leading-dash",
            ".hidden",
            "name\nunknown provider profile `x`",
            "ünï",
            long.as_str(),
        ] {
            let err = resolve_with(hostile, Path::new("/unused"), &|name| {
                panic!("environment must not be read for a refused name (asked for {name})")
            })
            .unwrap_err()
            .to_string();
            assert!(
                err.starts_with("invalid provider profile name "),
                "{hostile:?}: {err}"
            );
            assert!(!err.contains('\n'), "{hostile:?}: {err}");
            assert!(err.len() < 300, "{hostile:?}: {err}");
        }
        assert!(is_profile_name("ollama-anthropic"));
        assert!(is_profile_name("Local_2.5"));
        assert!(is_profile_name(&"a".repeat(PROFILE_NAME_MAX_LEN)));
    }

    #[test]
    fn malformed_profiles_file_is_an_error_even_for_builtins() {
        let path = profiles_file("malformed", "[providers.x\nkind = ");
        for requested in ["external", "x"] {
            let err = resolve_in(requested, &path, &[]).unwrap_err().to_string();
            assert!(err.starts_with("parse error in "), "{err}");
            assert!(err.contains(path.to_str().unwrap()), "{err}");
        }
        // An unreadable path (a directory) is an io error, not "unknown".
        let dir = temp_dir("is-a-dir");
        let err = resolve_in("x", &dir, &[]).unwrap_err().to_string();
        assert!(err.starts_with("io error at "), "{err}");
    }

    #[test]
    fn relative_profiles_path_is_refused() {
        for requested in ["external", "anything"] {
            let err = resolve_with(
                requested,
                Path::new("/unused"),
                &env_of(&[(PROFILES_ENV, "providers.toml")]),
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("providers.toml is not absolute"), "{err}");
            assert!(err.contains(PROFILES_ENV), "{err}");
        }
    }

    // ---- profiles_path -------------------------------------------------

    #[test]
    fn profiles_path_precedence() {
        let path = |pairs: &[(&str, &str)]| profiles_path_with(&env_of(pairs));
        assert_eq!(
            path(&[(PROFILES_ENV, "/etc/ru/p.toml"), ("HOME", "/home/u")]),
            Some(PathBuf::from("/etc/ru/p.toml"))
        );
        assert_eq!(
            path(&[("HOME", "/home/u")]),
            Some(PathBuf::from("/home/u/.config/ruharness/providers.toml"))
        );
        // Empty values count as unset.
        assert_eq!(
            path(&[(PROFILES_ENV, ""), ("HOME", "/home/u")]),
            Some(PathBuf::from("/home/u/.config/ruharness/providers.toml"))
        );
        assert_eq!(
            path(&[("HOME", ""), ("USERPROFILE", "/users/u")]),
            Some(PathBuf::from("/users/u/.config/ruharness/providers.toml"))
        );
        assert_eq!(path(&[]), None);
    }

    // ---- the real process environment ----------------------------------

    #[test]
    fn process_env_selects_the_profiles_file() {
        let server = serve_once(response(200, "OK", OK_BODY));
        let path = profiles_file(
            "process-env",
            &format!(
                "[providers.local]\nkind = \"anthropic\"\nbase_url = \"{}\"\n\
                 api_key_env = \"RUHARNESS_TEST_PROCESS_ENV_TOKEN\"\ncontext_tokens = 4096\n",
                server.base_url
            ),
        );
        let _env = EnvGuard::set(&[
            (PROFILES_ENV, path.to_str().unwrap()),
            ("RUHARNESS_TEST_PROCESS_ENV_TOKEN", "tok-from-env"),
        ]);

        assert_eq!(profiles_path(), Some(path.clone()));
        let got = resolve("local", Path::new("/unused")).unwrap();
        assert_eq!(got.profile, "local");
        assert_eq!(got.kind, "anthropic");
        assert_eq!(got.context_tokens, Some(4096));
        assert!(got.live);
        got.adapter.complete(&req()).unwrap();
        let seen = server.finish();
        assert_eq!(seen[0].header("x-api-key"), Some("tok-from-env"));
    }

    #[test]
    fn process_env_unknown_profile_names_the_file() {
        let path = profiles_file("process-env-unknown", EXAMPLE);
        let _env = EnvGuard::set(&[(PROFILES_ENV, path.to_str().unwrap())]);
        let err = resolve("not-there", Path::new("/unused"))
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("unknown provider profile `not-there`"),
            "{err}"
        );
        assert!(err.contains(path.to_str().unwrap()), "{err}");

        // Built-ins resolve with the file present, from the real env too.
        let got = resolve("replay", Path::new("/unused")).unwrap();
        assert_eq!((got.kind.as_str(), got.live), ("replay", false));
    }

    // ---- checked_complete ------------------------------------------------

    /// An adapter answering with a fixed `input_tokens`, counting its calls.
    struct Fixed {
        input_tokens: u64,
        calls: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl ProviderAdapter for Fixed {
        fn name(&self) -> &'static str {
            "fixed"
        }
        fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, Error> {
            self.calls.set(self.calls.get() + 1);
            Ok(CompletionResponse {
                text: "ok".into(),
                input_tokens: self.input_tokens,
                output_tokens: 1,
                stop_reason: "end_turn".into(),
            })
        }
    }

    fn fixed(
        input_tokens: u64,
        context_tokens: Option<u32>,
        live: bool,
    ) -> (ResolvedProvider, std::rc::Rc<std::cell::Cell<usize>>) {
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let provider = ResolvedProvider {
            adapter: Box::new(Fixed {
                input_tokens,
                calls: std::rc::Rc::clone(&calls),
            }),
            profile: "p".into(),
            kind: "anthropic".into(),
            context_tokens,
            live,
        };
        (provider, calls)
    }

    /// A request with a 6000-byte prompt and a 1000-token budget.
    fn big_req() -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            system: "s".repeat(3000),
            user: "u".repeat(3000),
            max_tokens: 1000,
        }
    }

    #[test]
    fn checked_complete_refuses_before_the_call_when_the_prompt_cannot_fit() {
        // 6000 / 3 + 1000 = 3000 tokens needed.
        let (provider, calls) = fixed(0, Some(2999), true);
        let err = checked_complete(&provider, &big_req()).unwrap_err();
        assert!(err.to_string().starts_with(PROMPT_DOES_NOT_FIT), "{err}");
        assert!(err.to_string().contains("profile `p`"), "{err}");
        assert!(is_context_error(&err));
        assert_eq!(calls.get(), 0, "nothing was sent");

        let (provider, calls) = fixed(0, Some(3000), true);
        checked_complete(&provider, &big_req()).unwrap();
        assert_eq!(calls.get(), 1);

        // No declared window: no preflight.
        let (provider, _) = fixed(0, None, true);
        checked_complete(&provider, &big_req()).unwrap();
    }

    #[test]
    fn checked_complete_detects_server_side_truncation_for_every_provider() {
        // 6000 / 6 = 1000 input tokens are the floor; 0 = not reported.
        for live in [true, false] {
            let (provider, calls) = fixed(999, None, live);
            let err = checked_complete(&provider, &big_req()).unwrap_err();
            assert!(err.to_string().starts_with(PROMPT_TRUNCATED), "{err}");
            assert!(matches!(err, Error::Invariant(_)));
            assert!(is_context_error(&err));
            assert_eq!(calls.get(), 1);

            for plausible in [0, 1000, 1500] {
                let (provider, _) = fixed(plausible, None, live);
                let response = checked_complete(&provider, &big_req()).unwrap();
                assert_eq!(response.input_tokens, plausible);
            }
        }
    }

    #[test]
    fn checked_complete_propagates_adapter_errors_unchanged() {
        let dir = temp_dir("checked-external");
        let provider = ResolvedProvider {
            adapter: Box::new(TraceAdapter::new(&dir, true)),
            profile: "external".into(),
            kind: "external".into(),
            context_tokens: None,
            live: false,
        };
        let err = checked_complete(&provider, &req()).unwrap_err();
        assert!(err.to_string().starts_with("awaiting response: "), "{err}");
        assert!(!is_context_error(&err));
    }
}
