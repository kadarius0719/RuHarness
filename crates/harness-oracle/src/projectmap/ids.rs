//! The map's ids (docs/PROJECT-MAP-DESIGN.md §3.3): made by the harness from
//! paths, never by a model. A program is `t-<stem>`, a library
//! `l-<stem of its first file>`: the file name without `.c`, lowercased, every
//! character outside `[a-z0-9_-]` turned into `-`. When two collide the whole
//! relative folder joins in (`t-<folder>-<stem>`, its `/` turned into `-`),
//! the part after the prefix cut to 60 characters, then `-2`, `-3`… until
//! unique. A program at the path of an accepted tool keeps that tool's id.

use harness_core::config::is_tool_id;
use std::collections::{BTreeMap, BTreeSet};

/// The longest part after `t-`/`l-` before a `-2`, `-3`… suffix.
pub const ID_BODY_MAX: usize = 60;
/// The longest part after `t-`/`l-`, suffix included (the tool-id
/// pattern's `{1,64}`): a body is cut to `64 -` the suffix's length.
pub const ID_MAX: usize = 64;

/// `text` with every character outside `[a-z0-9_-]` turned into `-`, ASCII
/// letters lowercased, and `/` turned into `-` like any other.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '_' | '-' => c,
            'A'..='Z' => c.to_ascii_lowercase(),
            _ => '-',
        })
        .collect()
}

/// At most [`ID_BODY_MAX`] characters (all ASCII after [`clean`]).
fn cut(body: String) -> String {
    let mut body = body;
    body.truncate(ID_BODY_MAX);
    if body.is_empty() {
        body.push('-');
    }
    body
}

/// A file's stem: its last path part without `.c`.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".c").unwrap_or(name)
}

/// The short form `<prefix><stem>`.
fn short(prefix: &str, path: &str) -> String {
    format!("{prefix}{}", cut(clean(stem(path))))
}

/// The long form `<prefix><folder>-<stem>` (the short form for a file at
/// the root).
fn long(prefix: &str, path: &str) -> String {
    match path.rsplit_once('/') {
        Some((folder, _)) => format!(
            "{prefix}{}",
            cut(format!("{}-{}", clean(folder), clean(stem(path))))
        ),
        None => short(prefix, path),
    }
}

/// An id for each of `paths` under `prefix` (`t-` or `l-`), keyed by path.
/// `accepted` holds `(path, id)` pairs: a path listed there keeps its id when
/// the id has this prefix and passes [`is_tool_id`]. Deterministic: the
/// same paths and accepted pairs give the same ids in any input order.
pub fn assign(
    prefix: &str,
    paths: &[String],
    accepted: &[(String, String)],
) -> BTreeMap<String, String> {
    let paths: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for (path, id) in accepted {
        if paths.contains(path.as_str())
            && id.starts_with(prefix)
            && is_tool_id(id)
            && !taken.contains(id)
            && !out.contains_key(path)
        {
            out.insert(path.clone(), id.clone());
            taken.insert(id.clone());
        }
    }
    let rest: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|p| !out.contains_key(*p))
        .collect();
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for p in &rest {
        *count.entry(short(prefix, p)).or_default() += 1;
    }
    for p in rest {
        let s = short(prefix, p);
        let base = if count[&s] > 1 || taken.contains(&s) {
            long(prefix, p)
        } else {
            s
        };
        let mut id = base.clone();
        let mut n = 2;
        while taken.contains(&id) {
            // The body is cut to make room for the suffix, so the id stays
            // within the tool-id pattern's 64 characters after the prefix
            // however many collide.
            let suffix = format!("-{n}");
            let mut body = base[prefix.len()..].to_string();
            body.truncate(ID_MAX - suffix.len());
            id = format!("{prefix}{body}{suffix}");
            n += 1;
        }
        debug_assert!(is_tool_id(&id), "{id} is a tool id");
        taken.insert(id.clone());
        out.insert(p.to_string(), id);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(paths: &[&str], accepted: &[(&str, &str)]) -> Vec<(String, String)> {
        let paths: Vec<String> = paths.iter().map(|p| (*p).to_string()).collect();
        let accepted: Vec<(String, String)> = accepted
            .iter()
            .map(|(p, i)| ((*p).to_string(), (*i).to_string()))
            .collect();
        assign("t-", &paths, &accepted).into_iter().collect()
    }

    fn pair(p: &str, i: &str) -> (String, String) {
        (p.to_string(), i.to_string())
    }

    #[test]
    fn a_stem_outside_the_rule_is_cleaned() {
        assert_eq!(
            ids(&["tools/My Tool+v2.c"], &[]),
            [pair("tools/My Tool+v2.c", "t-my-tool-v2")]
        );
        let long_name = format!("{}.c", "A".repeat(80));
        let got = ids(&[&long_name], &[]);
        assert_eq!(got[0].1, format!("t-{}", "a".repeat(60)));
        assert!(is_tool_id(&got[0].1));
    }

    #[test]
    fn two_colliding_stems_take_their_folders_then_numbers() {
        assert_eq!(
            ids(&["examples/Main.c", "tests/main.c", "util.c"], &[]),
            [
                pair("examples/Main.c", "t-examples-main"),
                pair("tests/main.c", "t-tests-main"),
                pair("util.c", "t-util"),
            ]
        );
        // The folder forms collide too (`a/b-c` and `a-b/c`): numbered.
        assert_eq!(
            ids(&["a/b-c/x.c", "a-b/c/x.c"], &[]),
            [
                pair("a-b/c/x.c", "t-a-b-c-x"),
                pair("a/b-c/x.c", "t-a-b-c-x-2")
            ]
        );
    }

    /// Past 999 collisions the suffix grows to five characters: the body is
    /// cut to make room, so every id stays a tool id (64 after the prefix).
    #[test]
    fn a_thousand_collisions_stay_within_the_tool_id_pattern() {
        let a = "a".repeat(60);
        let paths: Vec<String> = (0..1100).map(|n| format!("{a}{n}.c")).collect();
        let got = assign("t-", &paths, &[]);
        let ids: BTreeSet<&String> = got.values().collect();
        assert_eq!(ids.len(), paths.len(), "unique");
        for id in got.values() {
            assert!(is_tool_id(id), "{id} ({} long)", id.len());
        }
        assert!(got.values().any(|id| id.ends_with("-1000")));
    }

    #[test]
    fn an_accepted_tool_keeps_its_id() {
        assert_eq!(
            ids(
                &["examples/main.c", "tests/main.c"],
                &[("tests/main.c", "t-main")]
            ),
            [
                pair("examples/main.c", "t-examples-main"),
                pair("tests/main.c", "t-main"),
            ]
        );
        // An accepted id that breaks the rule is not kept.
        assert_eq!(ids(&["x.c"], &[("x.c", "t-Bad Id")]), [pair("x.c", "t-x")]);
    }
}
