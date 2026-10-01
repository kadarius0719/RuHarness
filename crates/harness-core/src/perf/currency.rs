//! Whether a perf row is current (docs/PERF-DESIGN.md §3.9 *Current*, §3.2
//! *Currency is judged per unit*): each input against today's, each with its
//! own reason in words. What could not be checked here (the computer, the
//! compilers) is said as such, never as "current".

use super::results::{Computer, Row, RowKind};

/// What today's inputs are, as far as the reader could learn them.
pub struct Today<'a> {
    /// The workload's digest today; `None` when the workload is gone or its
    /// input cannot be read.
    pub workload: Option<&'a str>,
    /// The C program's digest today.
    pub program: &'a str,
    /// A unit's crate digest today (`None`: no crate).
    pub crate_digest: &'a dyn Fn(&str) -> Option<String>,
    /// The unit's `replaces` today (unit rows).
    pub replaces: Option<&'a [String]>,
    /// The program's name today.
    pub program_name: &'a str,
    /// The units measurable today, in plan order (as-it-stands rows): `None`
    /// when not judged.
    pub measurable: Option<&'a [String]>,
    /// The computer today; `None`: not checked.
    pub computer: Option<&'a Computer>,
    /// `cc --version` and `rustc -V` today; `None`: not checked.
    pub compilers: Option<(&'a str, &'a str)>,
}

/// Why `row` is out of date, in words, one reason each; empty when it is
/// current as far as `today` could tell.
pub fn out_of_date(row: &Row, kind: RowKind, today: &Today<'_>) -> Vec<String> {
    let i = &row.inputs;
    let mut why = Vec::new();
    match today.workload {
        Some(w) if w == i.workload => {}
        Some(_) => why.push("your workload changed".to_string()),
        None => why.push("the workload is gone or its input cannot be read".to_string()),
    }
    if i.program != today.program {
        why.push("the C changed".to_string());
    }
    if i.program_name != today.program_name {
        why.push("the program's name changed".to_string());
    }
    if i.recipe != super::PERF_RECIPE {
        why.push("measured another way (an older perf)".to_string());
    }
    if i.launcher != super::PERF_LAUNCHER {
        why.push("measured by another launcher".to_string());
    }
    if kind == RowKind::Unit {
        for c in i.crates.iter().flatten() {
            if (today.crate_digest)(&c.id).as_deref() != Some(c.digest.as_str()) {
                why.push(format!("{}'s Rust changed since", c.id));
            }
        }
        if let (Some(then), Some(now)) = (&i.replaces, today.replaces) {
            if then.as_slice() != now {
                why.push("its replaced files changed".to_string());
            }
        }
    }
    if kind == RowKind::AsItStands {
        let held: Vec<&str> = i.units.iter().flatten().map(|u| u.id.as_str()).collect();
        for u in i.units.iter().flatten() {
            if (today.crate_digest)(&u.id).as_deref() != Some(u.crate_digest.as_str()) {
                why.push(format!("{}'s Rust changed since", u.id));
            }
        }
        if let Some(now) = today.measurable {
            for id in &held {
                if !now.iter().any(|n| n == id) {
                    why.push(format!("{id} is left out now"));
                }
            }
            let left: Vec<&str> = i.left_out.iter().flatten().map(|l| l.id.as_str()).collect();
            for id in now {
                if held.contains(&id.as_str()) {
                    continue;
                }
                match i
                    .left_out
                    .iter()
                    .flatten()
                    .find(|l| l.id == *id)
                    .map(|l| l.reason.as_str())
                {
                    None if !left.contains(&id.as_str()) => {
                        why.push(format!("{id} was accepted since"))
                    }
                    Some("not-fresh") | Some("accept-interrupted") | Some("replaces-changed") => {
                        why.push(format!("{id} was verified since"))
                    }
                    Some(_) => {
                        if let Some(l) = i.left_out.iter().flatten().find(|l| l.id == *id) {
                            if (today.crate_digest)(id).as_deref() != Some(l.crate_digest.as_str())
                            {
                                why.push(format!("{id}'s Rust changed since"));
                            }
                        }
                    }
                    None => {}
                }
            }
            let order_now: Vec<&str> = now
                .iter()
                .map(String::as_str)
                .filter(|id| held.contains(id))
                .collect();
            if order_now.len() == held.len() && order_now != held {
                why.push("the plan's order changed".to_string());
            }
        }
    }
    if let Some(c) = today.computer {
        if *c != i.computer {
            why.push(format!(
                "measured on another computer ({}, {} {})",
                crate::text::safe_line(&i.computer.cpu),
                crate::text::safe_line(&i.computer.os),
                crate::text::safe_line(&i.computer.build)
            ));
        }
    }
    if let Some((cc, rustc)) = today.compilers {
        let rust_differs = i.compilers.rustc.as_deref().is_some_and(|r| r != rustc);
        if i.compilers.cc != cc || rust_differs {
            why.push("measured with other compilers".to_string());
        }
    }
    why
}

#[cfg(test)]
mod tests {
    use super::super::results::{CrateDigest, LeftOut, RowInputs, UnitRef};
    use super::*;

    fn d(c: char) -> String {
        format!("blake3:{}", c.to_string().repeat(64))
    }

    fn row(kind: RowKind) -> Row {
        Row {
            workload: "w".into(),
            outcome: "too-short".into(),
            short: None,
            runs: None,
            platform_metrics: None,
            inputs: RowInputs {
                workload: d('a'),
                program: d('b'),
                crates: (kind == RowKind::Unit).then(|| {
                    vec![CrateDigest {
                        id: "u001".into(),
                        digest: d('c'),
                    }]
                }),
                replaces: (kind == RowKind::Unit).then(|| vec!["src/a.c".into()]),
                program_name: "tool".into(),
                units: (kind == RowKind::AsItStands).then(|| {
                    vec![
                        UnitRef {
                            id: "u001".into(),
                            crate_digest: d('c'),
                        },
                        UnitRef {
                            id: "u002".into(),
                            crate_digest: d('e'),
                        },
                    ]
                }),
                left_out: (kind == RowKind::AsItStands).then(|| {
                    vec![LeftOut {
                        id: "u003".into(),
                        crate_digest: String::new(),
                        reason: "not-fresh".into(),
                    }]
                }),
                recipe: super::super::PERF_RECIPE.into(),
                launcher: super::super::PERF_LAUNCHER.into(),
                computer: Computer {
                    os: "15.6".into(),
                    build: "24G84".into(),
                    arch: "arm64".into(),
                    cpu: "Apple M3".into(),
                    two_kinds: true,
                    fast_cores: 4,
                },
                compilers: super::super::results::Compilers {
                    cc: "cc 1".into(),
                    rustc: Some("rustc 1".into()),
                },
            },
            c: None,
            other: None,
            std: None,
            fat_lto: None,
            profile: None,
            step1: None,
            failed_run: None,
            setup: None,
            first_difference: None,
            found_before: None,
            last_try: None,
        }
    }

    #[test]
    fn each_input_has_its_own_reason() {
        let unit = row(RowKind::Unit);
        let crates = |id: &str| (id == "u001").then(|| d('c'));
        let replaces = vec!["src/a.c".to_string()];
        let a = d('a');
        let b = d('b');
        let today = Today {
            workload: Some(&a),
            program: &b,
            crate_digest: &crates,
            replaces: Some(&replaces),
            program_name: "tool",
            measurable: None,
            computer: Some(&unit.inputs.computer),
            compilers: Some(("cc 1", "rustc 1")),
        };
        assert!(out_of_date(&unit, RowKind::Unit, &today).is_empty());
        let other = d('x');
        let changed = |id: &str| (id == "u001").then(|| d('z'));
        let moved = vec!["src/b.c".to_string()];
        let today = Today {
            workload: Some(&other),
            program: &other,
            crate_digest: &changed,
            replaces: Some(&moved),
            program_name: "other",
            measurable: None,
            computer: None,
            compilers: Some(("cc 2", "rustc 1")),
        };
        let why = out_of_date(&unit, RowKind::Unit, &today);
        for want in [
            "your workload changed",
            "the C changed",
            "the program's name changed",
            "u001's Rust changed since",
            "its replaced files changed",
            "measured with other compilers",
        ] {
            assert!(why.iter().any(|w| w == want), "{want}: {why:?}");
        }
    }

    #[test]
    fn the_program_as_it_stands_is_judged_per_unit() {
        let p = row(RowKind::AsItStands);
        let crates = |id: &str| match id {
            "u001" => Some(d('c')),
            "u002" => Some(d('y')),
            _ => Some(d('q')),
        };
        let now = vec!["u002".to_string(), "u003".to_string(), "u004".to_string()];
        let (a, b) = (d('a'), d('b'));
        let today = Today {
            workload: Some(&a),
            program: &b,
            crate_digest: &crates,
            replaces: None,
            program_name: "tool",
            measurable: Some(&now),
            computer: None,
            compilers: None,
        };
        let why = out_of_date(&p, RowKind::AsItStands, &today);
        for want in [
            "u002's Rust changed since",
            "u001 is left out now",
            "u003 was verified since",
            "u004 was accepted since",
        ] {
            assert!(why.iter().any(|w| w == want), "{want}: {why:?}");
        }
    }
}
