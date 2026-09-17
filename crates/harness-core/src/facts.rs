//! The language-neutral fact model and its canonical JSONL serialization
//! (docs/SCHEMAS.md "facts.jsonl"). Scanner-output-only: `store` regenerates
//! the whole file and is byte-idempotent for identical input.

use crate::error::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// Version of the facts schema this build reads and writes.
pub const FACTS_SCHEMA_VERSION: u64 = 1;
/// Value of the frozen `schema` preamble field.
pub const FACTS_SCHEMA_NAME: &str = "ruharness-facts";

/// A source file record. Field order is canonical.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRecord {
    /// Repo-relative path (relative to the target root).
    pub path: String,
    /// Rendered content hash (`blake3:<hex>`).
    pub hash: String,
    /// Project-local includes, resolved repo-relative, sorted.
    pub includes: Vec<String>,
}

/// A symbol definition. Field order is canonical.
///
/// `name` is the frontend's canonical unique identifier (C: linkage name for
/// external symbols, `<file>::<name>` for statics). `kind` and `visibility`
/// are open string enums — readers must pass through unknown values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolRecord {
    /// Canonical unique identifier.
    pub name: String,
    /// Symbol kind (`function` at v1; `type|global|macro` reserved).
    pub kind: String,
    /// Defining file, repo-relative.
    pub file: String,
    /// `public` (reachable outside its defining file/module) or `internal`.
    pub visibility: String,
    /// Display signature in source-language syntax (informational).
    pub signature: String,
    /// 1-based (start_line, end_line) span in the defining file.
    pub span: (u32, u32),
}

/// A reference edge. Field order is canonical.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefRecord {
    /// Canonical id of the referencing symbol.
    pub from: String,
    /// File the reference occurs in, repo-relative.
    pub file: String,
    /// Canonical id of the target when `resolved`, else the raw source name.
    pub to: String,
    /// Reference kind (`call` at v1; others reserved).
    pub refkind: String,
    /// Whether `to` names a symbol defined in the project.
    pub resolved: bool,
}

/// The in-memory fact model: everything the scanner learned about a target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// Frontend that produced these facts (e.g. `c-tree-sitter`).
    pub frontend: String,
    /// Source files, sorted by path.
    pub files: Vec<FileRecord>,
    /// Symbols, sorted by (file, name).
    pub symbols: Vec<SymbolRecord>,
    /// Reference edges, sorted by (file, from, to, refkind).
    pub refs: Vec<RefRecord>,
}

#[derive(Serialize)]
struct HeaderOut<'a> {
    k: &'static str,
    schema: &'static str,
    schema_version: u64,
    frontend: &'a str,
}

#[derive(Serialize)]
struct RecordOut<'a, T: Serialize> {
    k: &'static str,
    #[serde(flatten)]
    record: &'a T,
}

impl Facts {
    /// Serialize to canonical JSONL bytes: header line, then `file`, `symbol`,
    /// `ref` records, each group sorted by its per-kind key with the full
    /// canonical line as the final tiebreak. Trailing newline. Byte-identical
    /// for identical input.
    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, Error> {
        fn lines<T: Serialize, K: Ord>(
            kind: &'static str,
            records: &[T],
            key: impl Fn(&T) -> K,
        ) -> Result<Vec<String>, Error> {
            let mut out: Vec<(K, String)> = Vec::with_capacity(records.len());
            for r in records {
                let line = serde_json::to_string(&RecordOut { k: kind, record: r })
                    .map_err(|e| Error::Invariant(format!("serialize {kind}: {e}")))?;
                out.push((key(r), line));
            }
            out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            Ok(out.into_iter().map(|(_, l)| l).collect())
        }

        let header = serde_json::to_string(&HeaderOut {
            k: "header",
            schema: FACTS_SCHEMA_NAME,
            schema_version: FACTS_SCHEMA_VERSION,
            frontend: &self.frontend,
        })
        .map_err(|e| Error::Invariant(format!("serialize header: {e}")))?;

        let mut all = vec![header];
        all.extend(lines("file", &self.files, |f| f.path.clone())?);
        all.extend(lines("symbol", &self.symbols, |s| {
            (s.file.clone(), s.name.clone())
        })?);
        all.extend(lines("ref", &self.refs, |r| {
            (
                r.file.clone(),
                r.from.clone(),
                r.to.clone(),
                r.refkind.clone(),
            )
        })?);
        let mut bytes = all.join("\n").into_bytes();
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Write canonical JSONL to `path` (atomic whole-file replace).
    pub fn store(&self, path: &Path) -> Result<(), Error> {
        let bytes = self.to_canonical_bytes()?;
        crate::ledger::write_atomic(path, &bytes)
    }

    /// Load facts from JSONL. Unknown record kinds and unknown fields are
    /// skipped per the open-schema rules; a newer `schema_version` is refused.
    pub fn load(path: &Path) -> Result<Facts, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let mut lines = text.lines();
        let header_line = lines
            .next()
            .ok_or_else(|| Error::parse(path, "empty facts file"))?;
        let header: serde_json::Value = serde_json::from_str(header_line)
            .map_err(|e| Error::parse(path, format!("header: {e}")))?;
        if header.get("schema").and_then(|v| v.as_str()) != Some(FACTS_SCHEMA_NAME) {
            return Err(Error::parse(path, "not a ruharness-facts file"));
        }
        let version = header
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::parse(path, "header missing schema_version"))?;
        if version > FACTS_SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                path: path.into(),
                found: version,
                supported: FACTS_SCHEMA_VERSION,
            });
        }
        let mut facts = Facts {
            frontend: header
                .get("frontend")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            ..Facts::default()
        };
        for (i, line) in lines.enumerate() {
            if line.is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| Error::parse(path, format!("line {}: {e}", i + 2)))?;
            let parse = |e: serde_json::Error| Error::parse(path, format!("line {}: {e}", i + 2));
            match value.get("k").and_then(|v| v.as_str()) {
                Some("file") => facts
                    .files
                    .push(serde_json::from_value(value).map_err(parse)?),
                Some("symbol") => facts
                    .symbols
                    .push(serde_json::from_value(value).map_err(parse)?),
                Some("ref") => facts
                    .refs
                    .push(serde_json::from_value(value).map_err(parse)?),
                _ => {} // unknown kind: pass over (open schema)
            }
        }
        Ok(facts)
    }

    /// Transitive project-local include closure of `start` files (paths are
    /// repo-relative). The result includes the start files themselves, sorted.
    pub fn include_closure(&self, start: &[String]) -> Vec<String> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = start.to_vec();
        while let Some(p) = stack.pop() {
            if !seen.insert(p.clone()) {
                continue;
            }
            if let Some(f) = self.files.iter().find(|f| f.path == p) {
                stack.extend(f.includes.iter().cloned());
            }
        }
        seen.into_iter().collect()
    }
}
