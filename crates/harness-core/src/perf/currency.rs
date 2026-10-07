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

/// Every reason's token (the MCP's closed set): the workload changed or is
/// gone, the C, the program's name, the recipe, the launcher, a unit's
/// Rust, its replaced files, a held unit left out, a unit accepted or
/// verified since, the plan's order, the computer, the compilers — and the
/// cockpit's own: measuring now, inputs too large to hash, an input perf
/// could not use.
pub const REASONS: &[&str] = &[
    "workload",
    "workload-gone",
    "program",
    "program-name",
    "recipe",
    "launcher",
    "rust",
    "replaces",
    "left-out",
    "accepted",
    "verified",
    "plan-order",
    "computer",
    "compilers",
    "measuring",
    "too-large",
    "input-unusable",
];

/// One reason a row is out of date: its token (one of [`REASONS`]) and its
/// words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    /// One of [`REASONS`].
    pub token: &'static str,
    /// In words.
    pub words: String,
}

fn reason(token: &'static str, words: impl Into<String>) -> Reason {
    Reason {
        token,
        words: words.into(),
    }
}

/// Why `row` is out of date, in words, one reason each; empty when it is
/// current as far as `today` could tell.
pub fn out_of_date(row: &Row, kind: RowKind, today: &Today<'_>) -> Vec<String> {
    reasons(row, kind, today)
        .into_iter()
        .map(|r| r.words)
        .collect()
}

/// [`out_of_date`] with each reason's token.
pub fn reasons(row: &Row, kind: RowKind, today: &Today<'_>) -> Vec<Reason> {
    let i = &row.inputs;
    let mut why = Vec::new();
    match today.workload {
        Some(w) if w == i.workload => {}
        Some(_) => why.push(reason("workload", "your workload changed")),
        None => why.push(reason(
            "workload-gone",
            "the workload is gone or its input cannot be read",
        )),
    }
    if i.program != today.program {
        why.push(reason("program", "the C changed"));
    }
    if i.program_name != today.program_name {
        why.push(reason("program-name", "the program's name changed"));
    }
    if i.recipe != super::PERF_RECIPE {
        why.push(reason("recipe", "measured another way (an older perf)"));
    }
    if i.launcher != super::PERF_LAUNCHER {
        why.push(reason("launcher", "measured by another launcher"));
    }
    if kind == RowKind::Unit {
        for c in i.crates.iter().flatten() {
            if (today.crate_digest)(&c.id).as_deref() != Some(c.digest.as_str()) {
                why.push(reason("rust", format!("{}'s Rust changed since", c.id)));
            }
        }
        if let (Some(then), Some(now)) = (&i.replaces, today.replaces) {
            if then.as_slice() != now {
                why.push(reason("replaces", "its replaced files changed"));
            }
        }
    }
    if kind == RowKind::AsItStands {
        let held: Vec<&str> = i.units.iter().flatten().map(|u| u.id.as_str()).collect();
        for u in i.units.iter().flatten() {
            if (today.crate_digest)(&u.id).as_deref() != Some(u.crate_digest.as_str()) {
                why.push(reason("rust", format!("{}'s Rust changed since", u.id)));
            }
        }
        if let Some(now) = today.measurable {
            for id in &held {
                if !now.iter().any(|n| n == id) {
                    why.push(reason("left-out", format!("{id} is left out now")));
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
                        why.push(reason("accepted", format!("{id} was accepted since")))
                    }
                    Some("not-fresh") | Some("accept-interrupted") | Some("replaces-changed") => {
                        why.push(reason("verified", format!("{id} was verified since")))
                    }
                    Some(_) => {
                        if let Some(l) = i.left_out.iter().flatten().find(|l| l.id == *id) {
                            if (today.crate_digest)(id).as_deref() != Some(l.crate_digest.as_str())
                            {
                                why.push(reason("rust", format!("{id}'s Rust changed since")));
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
                why.push(reason("plan-order", "the plan's order changed"));
            }
        }
    }
    if let Some(c) = today.computer {
        if *c != i.computer {
            why.push(reason(
                "computer",
                format!(
                    "measured on another computer ({}, {} {})",
                    crate::text::safe_line(&i.computer.cpu),
                    crate::text::safe_line(&i.computer.os),
                    crate::text::safe_line(&i.computer.build)
                ),
            ));
        }
    }
    if let Some((cc, rustc)) = today.compilers {
        let rust_differs = i.compilers.rustc.as_deref().is_some_and(|r| r != rustc);
        if i.compilers.cc != cc || rust_differs {
            why.push(reason("compilers", "measured with other compilers"));
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
    fn every_reason_has_a_closed_token() {
        let unit = row(RowKind::Unit);
        let changed = |_: &str| Some(d('z'));
        let moved = vec!["src/b.c".to_string()];
        let other = d('x');
        let computer = Computer {
            cpu: "Apple M4".into(),
            ..unit.inputs.computer.clone()
        };
        let today = Today {
            workload: Some(&other),
            program: &other,
            crate_digest: &changed,
            replaces: Some(&moved),
            program_name: "other",
            measurable: None,
            computer: Some(&computer),
            compilers: Some(("cc 2", "rustc 2")),
        };
        let why = reasons(&unit, RowKind::Unit, &today);
        assert_eq!(why.len(), 7, "{why:?}");
        assert!(why.iter().all(|r| REASONS.contains(&r.token)), "{why:?}");
    }

    #[test]
    fn an_input_that_comes_back_empty_is_a_change() {
        // perf could not read the input (it was missing) and stored the
        // workload's digest without it; today the file is there, empty.
        use super::super::results::SetupFacts;
        use super::super::workloads::{digest, Workload};
        let w = Workload {
            id: "w".into(),
            args: vec!["{input}".into()],
            input: Some("bench/empty.txt".into()),
            runs: 15,
        };
        let mut unusable = row(RowKind::CAlone);
        unusable.outcome = "input-unusable".into();
        unusable.setup = Some(SetupFacts {
            input: Some("missing".into()),
            ..SetupFacts::default()
        });
        unusable.inputs.workload = digest(&w, None);
        let crates = |_: &str| None;
        let empty = digest(&w, Some(b""));
        let b = d('b');
        let today = |workload| Today {
            workload,
            program: &b,
            crate_digest: &crates,
            replaces: None,
            program_name: "tool",
            measurable: None,
            computer: None,
            compilers: None,
        };
        let why = reasons(&unusable, RowKind::CAlone, &today(Some(&empty)));
        assert_eq!(
            why.iter().map(|r| r.token).collect::<Vec<_>>(),
            ["workload"],
            "{why:?}"
        );
        // Still missing: it says so, and nothing else.
        let why = reasons(&unusable, RowKind::CAlone, &today(None));
        assert_eq!(
            why.iter().map(|r| r.token).collect::<Vec<_>>(),
            ["workload-gone"]
        );
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

    fn tokens(why: Vec<Reason>) -> Vec<&'static str> {
        why.into_iter().map(|r| r.token).collect()
    }

    #[test]
    fn the_recipe_the_launcher_and_each_compiler_are_their_own_reason() {
        let unit = row(RowKind::Unit);
        let crates = |id: &str| (id == "u001").then(|| d('c'));
        let replaces = vec!["src/a.c".to_string()];
        let measurable = vec!["u001".to_string()];
        let (a, b) = (d('a'), d('b'));
        let today = |compilers| Today {
            workload: Some(&a),
            program: &b,
            crate_digest: &crates,
            replaces: Some(&replaces),
            program_name: "tool",
            // A unit's row is judged the same with the plan's measurable
            // units given (the cockpit gives them for every row).
            measurable: Some(&measurable),
            computer: Some(&unit.inputs.computer),
            compilers: Some(compilers),
        };
        let same = today(("cc 1", "rustc 1"));
        assert!(reasons(&unit, RowKind::Unit, &same).is_empty());
        let mut older = unit.clone();
        older.inputs.recipe = "perf-recipe-1".into();
        assert_eq!(tokens(reasons(&older, RowKind::Unit, &same)), ["recipe"]);
        let mut launched = unit.clone();
        launched.inputs.launcher = "perf-launcher-1".into();
        assert_eq!(
            tokens(reasons(&launched, RowKind::Unit, &same)),
            ["launcher"]
        );
        // Only rustc changed, or only cc: either is other compilers.
        for (cc, rustc) in [("cc 1", "rustc 2"), ("cc 2", "rustc 1")] {
            assert_eq!(
                tokens(reasons(&unit, RowKind::Unit, &today((cc, rustc)))),
                ["compilers"],
                "{cc}, {rustc}"
            );
        }
        // The C alone names no rustc: today's is not its concern.
        let mut c_alone = row(RowKind::CAlone);
        c_alone.inputs.compilers.rustc = None;
        let rustc_2 = today(("cc 1", "rustc 2"));
        assert!(reasons(&c_alone, RowKind::CAlone, &rustc_2).is_empty());
    }

    #[test]
    fn the_program_as_it_stands_says_exactly_what_changed() {
        // As perf run stores it: the held units' crates beside its units,
        // and the verified units it left out, each with its reason.
        let mut p = row(RowKind::AsItStands);
        p.inputs.crates = Some(vec![
            CrateDigest {
                id: "u001".into(),
                digest: d('c'),
            },
            CrateDigest {
                id: "u002".into(),
                digest: d('e'),
            },
        ]);
        let left = |id: &str, digest: String, reason: &str| LeftOut {
            id: id.into(),
            crate_digest: digest,
            reason: reason.into(),
        };
        p.inputs.left_out = Some(vec![
            left("u003", String::new(), "not-fresh"),
            left("u004", d('f'), "crate-does-not-build"),
            left("u005", d('g'), "replaces-changed"),
            left("u006", d('h'), "accept-interrupted"),
        ]);
        let unchanged = |id: &str| match id {
            "u001" => Some(d('c')),
            "u002" => Some(d('e')),
            "u004" => Some(d('f')),
            _ => None,
        };
        let u002_changed = |id: &str| match id {
            "u002" => Some(d('y')),
            id => unchanged(id),
        };
        let u004_changed = |id: &str| match id {
            "u004" => Some(d('z')),
            id => unchanged(id),
        };
        let (a, b) = (d('a'), d('b'));
        let judge = |crates: &dyn Fn(&str) -> Option<String>, now: &[&str]| {
            let now: Vec<String> = now.iter().map(|s| s.to_string()).collect();
            let today = Today {
                workload: Some(&a),
                program: &b,
                crate_digest: crates,
                replaces: None,
                program_name: "tool",
                measurable: Some(&now),
                computer: None,
                compilers: None,
            };
            reasons(&p, RowKind::AsItStands, &today)
        };
        // Nothing changed: current. A unit that does not build is still left
        // out today; its crate is the same.
        assert!(judge(&unchanged, &["u001", "u002"]).is_empty());
        assert!(judge(&unchanged, &["u001", "u002", "u004"]).is_empty());
        // One held crate changed: said once.
        assert_eq!(tokens(judge(&u002_changed, &["u001", "u002"])), ["rust"]);
        // The plan's order changed, nothing else.
        assert_eq!(tokens(judge(&unchanged, &["u002", "u001"])), ["plan-order"]);
        // A held unit left out now: that alone (the order of the rest is
        // not a change).
        assert_eq!(tokens(judge(&unchanged, &["u002"])), ["left-out"]);
        // The crate of a unit left out because it did not build changed.
        let why = judge(&u004_changed, &["u001", "u002", "u004"]);
        assert_eq!(
            why,
            [Reason {
                token: "rust",
                words: "u004's Rust changed since".into(),
            }]
        );
        // Units left out as not fresh, replaces changed or Accept
        // interrupted are measurable now: each was verified since.
        let why = judge(&unchanged, &["u001", "u002", "u003", "u005", "u006"]);
        assert_eq!(
            why.iter().map(|r| r.words.as_str()).collect::<Vec<_>>(),
            [
                "u003 was verified since",
                "u005 was verified since",
                "u006 was verified since"
            ]
        );
    }
}
