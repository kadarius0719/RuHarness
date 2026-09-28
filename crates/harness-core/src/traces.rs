//! Trace files of the trace-backed providers (docs/SCHEMAS.md "M3
//! additions"): `<key>.request.json` / `<key>.response.json`, keyed by the
//! request alone. The read-only half lives here so every reader — the
//! executor, the benchmark, harness-mcp's `harness_request` — checks a
//! recorded turn the same way; the adapters that WRITE traces stay in
//! harness-llm.

use crate::error::Error;
use crate::traits::{CompletionRequest, CompletionResponse};
use std::path::{Path, PathBuf};

/// Largest trace file [`load_recorded`] reads.
pub const MAX_TRACE_BYTES: u64 = 16 * 1024 * 1024;

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
    let actual = request_key(&request)?;
    if actual != key {
        return Err(Error::Invariant(format!(
            "{} hashes to request key {actual}, not {key}: the recorded request was altered",
            req_path.display()
        )));
    }
    let (resp_path, resp_text) = read("response")?;
    let response: CompletionResponse =
        serde_json::from_str(&resp_text).map_err(|e| Error::parse(&resp_path, e.to_string()))?;
    Ok((request, response))
}

/// Whether `key` is a trace key: exactly 8 lowercase hex digits (checked
/// before it becomes a path component).
pub fn is_trace_key(key: &str) -> bool {
    key.len() == 8
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The pending request of a hand-off (docs/CHAT-PANE-DESIGN.md §4.4
/// `harness_request`): `<dir>/<key>.request.json` with [`load_recorded`]'s
/// checks — `key` 8 lowercase hex, the dir and the file real (non-symlink)
/// entries of bounded size, the request re-serializing to `key` — and NO
/// `<key>.response.json` yet (a hand-off already answered is not pending).
pub fn load_pending(dir: &Path, key: &str) -> Result<CompletionRequest, Error> {
    if !is_trace_key(key) {
        return Err(Error::Invariant(format!(
            "request key {:?} is not 8 lowercase hex digits",
            key.chars().take(16).collect::<String>()
        )));
    }
    let regular = |path: &Path, want_dir: bool| -> Result<(), Error> {
        let meta = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
        let ok = if want_dir {
            meta.file_type().is_dir()
        } else {
            meta.file_type().is_file() && meta.len() <= MAX_TRACE_BYTES
        };
        if !ok {
            return Err(Error::Invariant(format!(
                "{} is not a regular {} of bounded size (symlinks are refused)",
                path.display(),
                if want_dir { "directory" } else { "file" }
            )));
        }
        Ok(())
    };
    regular(dir, true)?;
    let response = dir.join(format!("{key}.response.json"));
    match std::fs::symlink_metadata(&response) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io(&response, e)),
        Ok(_) => {
            return Err(Error::Invariant(format!(
                "the hand-off {key} already has a response: it is not pending"
            )))
        }
    }
    let path = dir.join(format!("{key}.request.json"));
    regular(&path, false)?;
    let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
    let request: CompletionRequest =
        serde_json::from_str(&text).map_err(|e| Error::parse(&path, e.to_string()))?;
    let actual = request_key(&request)?;
    if actual != key {
        return Err(Error::Invariant(format!(
            "{} hashes to request key {actual}, not {key}: the request was altered",
            path.display()
        )));
    }
    Ok(request)
}

/// `<dir>/<key>.response.json`.
pub fn response_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.response.json"))
}

/// File `response` as `<dir>/<key>.response.json` — a NEW file, never over
/// one (a hand-off answered once stays answered): pretty JSON written to a
/// temp dotfile (no trace reader looks at it), synced, then hard-linked into
/// place, so it is whole or absent. `AlreadyExists` is an error naming the
/// hand-off.
pub fn write_new_response(
    dir: &Path,
    key: &str,
    response: &CompletionResponse,
) -> Result<(), Error> {
    use std::io::Write;
    if !is_trace_key(key) {
        return Err(Error::Invariant(format!(
            "request key {:?} is not 8 lowercase hex digits",
            key.chars().take(16).collect::<String>()
        )));
    }
    let mut bytes = serde_json::to_vec_pretty(response)
        .map_err(|e| Error::Invariant(format!("serialize response: {e}")))?;
    bytes.push(b'\n');
    let name = format!("{key}.response.json");
    let target = dir.join(&name);
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::hard_link(&tmp, &target)
    })();
    let _ = std::fs::remove_file(&tmp);
    match written {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(Error::Invariant(format!(
            "the hand-off {key} already has a response ({}): it is not answered twice",
            target.display()
        ))),
        Err(e) => Err(Error::io(&target, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(user: &str) -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            system: "sys".into(),
            user: user.into(),
            max_tokens: 100,
        }
    }

    #[test]
    fn a_pending_request_is_checked_and_answered_once() {
        let dir = std::env::temp_dir().join(format!("ruharness-traces-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = req("translate this");
        let key = request_key(&r).unwrap();
        assert!(is_trace_key(&key), "{key}");
        let path = dir.join(format!("{key}.request.json"));
        std::fs::write(&path, serde_json::to_string_pretty(&r).unwrap()).unwrap();
        assert_eq!(load_pending(&dir, &key).unwrap().user, "translate this");
        // Not a key; not its request; a link.
        assert!(load_pending(&dir, "0123ABCD").is_err());
        assert!(load_pending(&dir, "../x").is_err());
        std::fs::write(&path, serde_json::to_string(&req("altered")).unwrap()).unwrap();
        assert!(load_pending(&dir, &key)
            .unwrap_err()
            .to_string()
            .contains("was altered"));
        std::fs::write(&path, serde_json::to_string(&r).unwrap()).unwrap();
        // Answered once, never over a response; then no longer pending.
        let response = CompletionResponse {
            text: "the reply".into(),
            input_tokens: 0,
            output_tokens: 0,
            stop_reason: "end_turn".into(),
        };
        write_new_response(&dir, &key, &response).unwrap();
        let (_, back) = load_recorded(&dir, &key).unwrap();
        assert_eq!(back.text, "the reply");
        assert!(write_new_response(&dir, &key, &response)
            .unwrap_err()
            .to_string()
            .contains("already has a response"));
        assert!(load_pending(&dir, &key)
            .unwrap_err()
            .to_string()
            .contains("not pending"));
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        #[cfg(unix)]
        {
            let other = req("other");
            let okey = request_key(&other).unwrap();
            let real = dir.join("real.json");
            std::fs::write(&real, serde_json::to_string(&other).unwrap()).unwrap();
            std::os::unix::fs::symlink(&real, dir.join(format!("{okey}.request.json"))).unwrap();
            assert!(load_pending(&dir, &okey)
                .unwrap_err()
                .to_string()
                .contains("symlinks are refused"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
