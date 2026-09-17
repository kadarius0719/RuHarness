//! Content hashing (docs/SCHEMAS.md "Hashes"): blake3, rendered as
//! `blake3:<64-hex>`; file-set hashes over sorted (path, file-hash) pairs.
//! Nothing is ever keyed by a commit SHA.

use crate::error::Error;
use std::path::Path;

/// Prefix carried by every rendered hash.
pub const HASH_PREFIX: &str = "blake3:";

/// Hash raw bytes, rendered as `blake3:<64-hex>`.
pub fn bytes_hash(bytes: &[u8]) -> String {
    format!("{HASH_PREFIX}{}", blake3::hash(bytes).to_hex())
}

/// Hash a file's contents, rendered as `blake3:<64-hex>`.
pub fn file_hash(path: &Path) -> Result<String, Error> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    Ok(bytes_hash(&bytes))
}

/// File-set hash: for the given (repo-relative path, per-file rendered hash)
/// pairs, sorted by path, hash the concatenation of `<path>\0<hex>\n` where
/// `<hex>` is the per-file hash with its `blake3:` prefix stripped.
pub fn file_set_hash(pairs: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = pairs.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = blake3::Hasher::new();
    for (path, hash) in sorted {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(hash.strip_prefix(HASH_PREFIX).unwrap_or(hash).as_bytes());
        hasher.update(b"\n");
    }
    format!("{HASH_PREFIX}{}", hasher.finalize().to_hex())
}

/// Compute the file-set hash of concrete files on disk. `paths` are
/// repo-relative; they are read relative to `root`.
pub fn file_set_hash_on_disk(root: &Path, paths: &[String]) -> Result<String, Error> {
    let mut pairs = Vec::with_capacity(paths.len());
    for p in paths {
        pairs.push((p.clone(), file_hash(&root.join(p))?));
    }
    Ok(file_set_hash(&pairs))
}

/// The `rust_crate` verdict digest (docs/SCHEMAS.md "Verdicts"): a file-set
/// hash over exactly `Cargo.toml`, `Cargo.lock` (when present), and every
/// non-dotfile under `src/` of the unit crate at `crate_dir`, paths
/// repo-relative to `root`. A closed file list: nothing else in the crate
/// directory affects the digest, so independent implementations can
/// reproduce committed digests and stray files cannot flip freshness.
pub fn unit_crate_file_set_hash(root: &Path, crate_dir: &Path) -> Result<String, Error> {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for name in ["Cargo.toml", "Cargo.lock"] {
        let p = crate_dir.join(name);
        if p.exists() {
            files.push(p);
        }
    }
    let src = crate_dir.join("src");
    if src.exists() {
        collect_files(&src, &mut files)?;
    }
    files.sort();
    let mut pairs = Vec::with_capacity(files.len());
    for f in &files {
        let rel = f
            .strip_prefix(root)
            .map_err(|_| {
                Error::Invariant(format!("{} escapes root {}", f.display(), root.display()))
            })?
            .to_string_lossy()
            .into_owned();
        pairs.push((rel, file_hash(f)?));
    }
    Ok(file_set_hash(&pairs))
}

fn collect_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue; // dotfiles (e.g. .DS_Store) never affect digests
        }
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}
