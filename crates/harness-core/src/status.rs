//! The per-unit staleness report behind `harness state status`
//! (docs/SCHEMAS.md "CLI contract"; docs/CLI-HARDENING.md §4): one struct
//! that the human line, the `--json` `unit` event, the review cockpit and
//! the MCP bridge all render from, so the four hash comparisons and the
//! contradiction rule live in exactly one place.

use crate::attempts;
use crate::error::Error;
use crate::hash;
use crate::ledger::{Holder, Ledger, WriterLock};
use crate::plan::{Plan, Unit, UnitStatus};
use crate::verdict::Verdict;
use crate::TargetContext;
use serde::Serialize;

/// Whether a unit's latest verdict exists and could be read.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VerdictState {
    /// `oracle-latest.json` loaded.
    Present,
    /// No verdict yet.
    Missing,
    /// A verdict file that does not parse (corrupt?).
    Unreadable,
}

/// The latest verdict, as far as staleness is concerned.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VerdictReport {
    /// Whether it exists and reads.
    pub state: VerdictState,
    /// Its colour (present only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub green: Option<bool>,
    /// Which of its inputs no longer match the tree: `source`, `rust-crate`,
    /// `driver`, `configuration` (present only; empty = fresh).
    pub stale: Vec<String>,
}

/// The start of the toolchain entry a file-list verdict records for its
/// configuration ([`configuration_entry`]).
pub const CONFIGURATION_ENTRY_PREFIX: &str = "configuration: ";

/// The toolchain entry a file-list target's verdict of `unit` records for
/// the configuration it was built under (the 2026-10-08 triage, decisions 5
/// and 12): `configuration: <name> <blake3 of the flags, then each of the
/// unit's listed files with its include folders>`, every path in its
/// resolved form (as a scan records it: `-Ix/` and `-Ix` are one flag), so
/// a file added to the tool or a cosmetic spelling stales no verdict.
/// `None` for the folder form, whose verdicts carry no such entry (they
/// stay byte for byte). A verdict whose entry differs from this one is
/// stale: a changed `-D` can change a struct's layout under the verified
/// Rust. (A header the unit reaches is the `source` digest's.)
pub fn configuration_entry(ctx: &TargetContext, unit: &Unit) -> Option<String> {
    let crate::config::Form::FileList(list) = &ctx.config.target.form else {
        return None;
    };
    let resolver = crate::sources::Resolver::of(ctx).ok().flatten();
    let mut bytes: Vec<u8> = Vec::new();
    let mut field = |s: &str| {
        bytes.extend_from_slice(s.as_bytes());
        bytes.push(0);
    };
    field("flags");
    let flags = match &resolver {
        Some(resolver) => resolver.resolved_flags(&list.configuration.flags),
        None => list.configuration.flags.clone(),
    };
    for flag in &flags {
        field(flag);
    }
    let files: Vec<(String, Vec<String>)> = match &resolver {
        Some(resolver) => resolver
            .listed()
            .iter()
            .map(|f| (f.path.clone(), f.include_dirs.clone()))
            .collect(),
        None => list
            .files
            .iter()
            .map(|f| (f.path.clone(), f.include_dirs.clone()))
            .collect(),
    };
    // The unit's files as the plan names them (root-relative, as a scan
    // records them, which the resolver's listed paths are).
    let mut own: Vec<&(String, Vec<String>)> = files
        .iter()
        .filter(|(path, _)| unit.files.contains(path))
        .collect();
    own.sort();
    own.dedup();
    for (path, dirs) in own {
        field("file");
        field(path);
        for dir in dirs {
            field(dir);
        }
    }
    Some(format!(
        "{CONFIGURATION_ENTRY_PREFIX}{} {}",
        crate::text::safe_line(&list.configuration.name),
        hash::bytes_hash(&bytes)
    ))
}

/// One recorded migrate attempt, for the list a client shows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AttemptSummary {
    /// The attempt id.
    pub id: String,
    /// Provider kind it ran under.
    pub provider_kind: String,
    /// Its outcome.
    pub outcome: String,
    /// Bound to the CURRENT unit source (the R-5 provenance rule).
    pub bound: bool,
}

/// Everything `harness state status` knows about one unit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UnitReport {
    /// The unit id.
    pub id: String,
    /// Plan status (`planned`, `in-progress`, `verified`, `merged`, …).
    pub status: String,
    /// The plan's `source_hash` still matches the tree.
    pub source_fresh: bool,
    /// The latest verdict.
    pub verdict: VerdictReport,
    /// Status and verdict evidence disagree (a done-claiming status without
    /// fresh green evidence, or fresh green evidence the status never
    /// absorbed) — and no writer is at work.
    pub contradiction: bool,
    /// A LIVE writer holds the ledger and the unit looked inconsistent: the
    /// verdict+status pair is being written right now
    /// (docs/CLI-HARDENING.md §1). Reported INSTEAD of `contradiction`; a
    /// dead holder's leftover line is ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_in_flight: Option<Holder>,
    /// A promotion of this unit was interrupted and no live writer is at
    /// work: the attempt id of its `.promote-<id>/` marker, or `legacy` for a
    /// bare `.<crate>.prev` (docs/TUI-DESIGN.md §2). The next writing command
    /// (`verify`, `migrate`, `promote`, `override`) recovers it by evidence;
    /// until then the unit is reported this way INSTEAD of `contradiction`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promotion_interrupted: Option<String>,
    /// The unit's recorded migrate attempts.
    pub attempts: Vec<AttemptSummary>,
    /// How its verdict covers today's features (docs/FEATURES-DESIGN.md §3):
    /// absent without a features file or without a verdict. A marker, never
    /// part of `stale` or `contradiction`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<crate::features::Coverage>,
    /// The verdict was written before the person adopted this folder
    /// ([`crate::adopt::made_before_adoption`]): made elsewhere until
    /// `harness verify` runs it here. A marker, never part of `stale` or
    /// `contradiction`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub made_elsewhere: bool,
}

impl UnitReport {
    /// The verdict is present, green and fresh.
    pub fn fresh_green(&self) -> bool {
        self.verdict.state == VerdictState::Present
            && self.verdict.green == Some(true)
            && self.verdict.stale.is_empty()
    }

    /// The human status line, exactly as `harness state status` prints it.
    pub fn render_line(&self) -> String {
        let desc = match self.verdict.state {
            VerdictState::Present => {
                let color = if self.verdict.green == Some(true) {
                    "green"
                } else {
                    "red"
                };
                if self.verdict.stale.is_empty() {
                    format!("{color} (fresh)")
                } else {
                    format!("{color} (STALE: {})", self.verdict.stale.join(", "))
                }
            }
            VerdictState::Missing => "no verdict".to_string(),
            VerdictState::Unreadable => "verdict UNREADABLE (corrupt?)".to_string(),
        };
        let tail = match &self.write_in_flight {
            Some(h) => format!(
                "  << write in flight (pid {}, `{}`) — re-check when it is done",
                h.pid, h.command
            ),
            None => match &self.promotion_interrupted {
                Some(id) => format!(
                    "  << promotion of {id} interrupted — the next writing command recovers it"
                ),
                None if self.contradiction => {
                    "  << CONTRADICTION: status and verdict evidence disagree".to_string()
                }
                None => String::new(),
            },
        };
        let desc = match &self.features {
            Some(crate::features::Coverage::Behind(reasons)) => {
                format!("{desc} features=behind({})", reasons.join(", "))
            }
            Some(crate::features::Coverage::Current) => format!("{desc} features=current"),
            None => desc,
        };
        let desc = if self.made_elsewhere {
            format!("{desc} made-elsewhere")
        } else {
            desc
        };
        format!(
            "status: {} [{}] plan={} verdict={desc}{tail}",
            self.id,
            self.status,
            if self.source_fresh {
                "fresh"
            } else {
                "SOURCE-STALE"
            },
        )
    }

    /// The human attempts line (`None` when there are no attempts).
    pub fn render_attempts_line(&self) -> Option<String> {
        if self.attempts.is_empty() {
            return None;
        }
        let bound = self.attempts.iter().filter(|a| a.bound).count();
        let summary: Vec<String> = self
            .attempts
            .iter()
            .map(|a| format!("{}:{}:{}", a.id, a.provider_kind, a.outcome))
            .collect();
        Some(format!(
            "status:   attempts: {} ({} bound to current source) [{}]",
            self.attempts.len(),
            bound,
            summary.join(", ")
        ))
    }
}

/// Compute the report for `unit`. Detection of a contradiction or a stale
/// input is two-phase: a lock-free reader's plan-then-verdict snapshot can
/// straddle a writer's non-atomic verdict+status pair, so a first hit is
/// re-checked once from fresh reads; if it holds and a writer holds the
/// ledger, the unit is reported `write_in_flight` instead
/// (docs/CLI-HARDENING.md §1 "What a lock-free reader can see").
///
/// `features` is today's digests ([`crate::features::FeaturesNow::compute`],
/// once per read), `None` without a features file.
pub fn unit_report(
    ctx: &TargetContext,
    ledger: &Ledger,
    facts: &crate::Facts,
    unit: &Unit,
    features: Option<&crate::features::FeaturesNow>,
) -> Result<UnitReport, Error> {
    let mut report = compute(ctx, ledger, facts, unit, features)?;
    if report.contradiction || !report.verdict.stale.is_empty() {
        // Phase two: the plan entry and the verdict, re-read.
        let plan = Plan::load(&ledger.plan_path())?;
        let again = match plan.units.iter().find(|u| u.id == unit.id) {
            Some(fresh_unit) => compute(ctx, ledger, facts, fresh_unit, features)?,
            None => report.clone(),
        };
        report = again;
        if report.contradiction || !report.verdict.stale.is_empty() {
            // A holder that died without cleanup (a signal death never
            // truncates the line) is diagnostics, not a writer at work.
            if let Some(holder) = live_holder(ledger)? {
                report.contradiction = false;
                report.write_in_flight = Some(holder);
            }
        }
    }
    // An interrupted promotion (marker left behind, nobody at work) explains
    // whatever the unit looks like until the next writer recovers it.
    if report.write_in_flight.is_none() {
        if let Some(marker) = promotion_marker(ledger, unit)? {
            if live_holder(ledger)?.is_none() {
                report.contradiction = false;
                report.promotion_interrupted = Some(marker);
            }
        }
    }
    Ok(report)
}

/// The ledger's writer-lock holder, when its process is alive (a dead
/// holder's leftover line is ignored). Reads one bounded file and probes one
/// pid — cheap enough for a client to call before offering a writing act.
pub fn live_holder(ledger: &Ledger) -> Result<Option<Holder>, Error> {
    Ok(WriterLock::holder(ledger)?.filter(|h| pid_alive(h.pid)))
}

/// The attempt id of the first `.promote-<id>/` marker in the unit dir, or
/// `legacy` for a bare `.<crate>.prev` (the pre-marker protocol). Public for
/// perf, which refuses to measure a unit whose Accept was interrupted
/// (docs/PERF-DESIGN.md §3.2).
pub fn promotion_marker(ledger: &Ledger, unit: &Unit) -> Result<Option<String>, Error> {
    let dir = ledger.unit_dir(&unit.id);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(&dir, e)),
    };
    let mut markers: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_prefix(".promote-"))
                .map(str::to_string)
        })
        .collect();
    markers.sort();
    if let Some(first) = markers.into_iter().next() {
        return Ok(Some(first));
    }
    let legacy = unit
        .oracle_param_str("rust_crate")
        .is_some_and(|name| dir.join(format!(".{name}.prev")).is_dir());
    Ok(legacy.then(|| "legacy".to_string()))
}

/// `kill -0 <pid>`: true while the process exists — the same unsafe-free
/// probe the oracle uses for its own process groups; it runs no target
/// code. A pid reused by an unrelated process reads as alive (the next
/// writer truncates the stale line anyway); a holder owned by another user
/// reads as dead (the ledger is single-user by design). If the probe itself
/// cannot run, assume alive: never invent a contradiction from a failed
/// probe.
fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(true)
}

fn compute(
    ctx: &TargetContext,
    ledger: &Ledger,
    facts: &crate::Facts,
    unit: &Unit,
    features: Option<&crate::features::FeaturesNow>,
) -> Result<UnitReport, Error> {
    // The files the unit's compile reads (a file list's resolver closure,
    // the 2026-10-08 triage, decision 11): what `verify` hashed.
    let closure = crate::sources::unit_closure(ctx, facts, &unit.files);
    let source_now = hash::file_set_hash_on_disk(&ctx.root, &closure)
        .unwrap_or_else(|_| "blake3:unreadable".into());
    let source_fresh = source_now == unit.source_hash;

    let mut coverage = None;
    let mut made_elsewhere = false;
    let verdict_path = ledger.verdict_latest_path(&unit.id);
    let verdict = match Verdict::load(&verdict_path) {
        Ok(v) => {
            coverage = features.map(|now| now.coverage(&v.inputs));
            made_elsewhere = crate::adopt::made_before_adoption(&ctx.root, &verdict_path);
            let mut stale: Vec<String> = Vec::new();
            if v.inputs.unit_source != source_now {
                stale.push("source".into());
            }
            if !v.inputs.rust_crate.is_empty() {
                if let Some(crate_dir) = unit.oracle_param_str("rust_crate") {
                    let dir = ledger.unit_dir(&unit.id).join(crate_dir);
                    let now = hash::unit_crate_file_set_hash(&ctx.root, &dir)
                        .unwrap_or_else(|_| "blake3:unreadable".into());
                    if now != v.inputs.rust_crate {
                        stale.push("rust-crate".into());
                    }
                }
            }
            if !v.inputs.driver.is_empty() {
                if let Some(driver) = unit.oracle_param_str("driver") {
                    let now = hash::file_hash(&ctx.root.join(driver))
                        .unwrap_or_else(|_| "blake3:unreadable".into());
                    if now != v.inputs.driver {
                        stale.push("driver".into());
                    }
                }
            }
            // The configuration a file-list verdict was built under: an
            // edited flag or folder, or a verdict that predates the entry.
            let recorded = v
                .inputs
                .toolchain
                .iter()
                .find(|t| t.starts_with(CONFIGURATION_ENTRY_PREFIX));
            if recorded.map(String::as_str) != configuration_entry(ctx, unit).as_deref() {
                stale.push("configuration".into());
            }
            VerdictReport {
                state: VerdictState::Present,
                green: Some(v.green),
                stale,
            }
        }
        Err(e) if e.is_not_found() => VerdictReport {
            state: VerdictState::Missing,
            green: None,
            stale: Vec::new(),
        },
        // Newer-schema refusals must surface, not read as "unreadable".
        Err(e @ Error::SchemaTooNew { .. }) => return Err(e),
        Err(_) => VerdictReport {
            state: VerdictState::Unreadable,
            green: None,
            stale: Vec::new(),
        },
    };

    // Verdicts are authoritative over plan status; flag both directions
    // (docs/SCHEMAS.md): a done-claiming status without FRESH green evidence
    // (red, or stale because crate/source/driver changed since), and fresh
    // green evidence the status never absorbed.
    let done_claimed = matches!(unit.status, UnitStatus::Verified | UnitStatus::Merged);
    let contradiction = match verdict.state {
        VerdictState::Present => {
            let green = verdict.green == Some(true);
            let fresh = verdict.stale.is_empty();
            (done_claimed && (!green || !fresh)) || (green && fresh && !done_claimed)
        }
        VerdictState::Missing | VerdictState::Unreadable => done_claimed,
    };

    let attempts = attempts::load_unit_attempts(ledger, &unit.id)?
        .into_iter()
        .map(|a| AttemptSummary {
            bound: a.unit_source == source_now,
            id: a.id,
            provider_kind: a.provider_kind,
            outcome: a.outcome,
        })
        .collect();

    Ok(UnitReport {
        id: unit.id.clone(),
        status: unit.status.as_str().to_string(),
        source_fresh,
        verdict,
        contradiction,
        write_in_flight: None,
        promotion_interrupted: None,
        attempts,
        features: coverage,
        made_elsewhere,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A target over `root` with this `harness.toml` text.
    fn target(root: &std::path::Path, body: &str) -> TargetContext {
        let config: crate::TargetConfig = toml::from_str(&format!("{body}\n")).expect("config");
        TargetContext {
            root: root.to_path_buf(),
            ledger: root.join("migration"),
            tool: None,
            config,
        }
    }

    /// A file-list verdict records its configuration; status reads an
    /// edited flag, an edited folder, or a verdict without the entry as a
    /// stale verdict (`configuration`). A folder-form verdict has none and
    /// needs none.
    #[test]
    fn a_changed_configuration_makes_a_file_list_verdict_stale() {
        let root = std::env::temp_dir().join(format!(
            "ruharness-status-config-{}-{}",
            std::process::id(),
            crate::hash::random_hex(4)
        ));
        std::fs::create_dir_all(root.join("lib")).expect("dir");
        std::fs::write(root.join("lib/a.c"), "int a;\n").expect("a.c");
        let files = |dirs: &str, flags: &str| {
            format!(
                "schema_version = 2\n[target]\nname = \"t\"\n\
                 files = [{{ path = \"lib/a.c\", include_dirs = [{dirs}] }}]\n\
                 configuration = {{ name = \"make\", from = \"stated\", flags = [{flags}] }}"
            )
        };
        let unit: Unit = toml::from_str(
            "id = \"u\"\nstatus = \"pending\"\nfiles = [\"lib/a.c\"]\nsymbols = [\"a\"]\n",
        )
        .expect("unit");
        let built = target(&root, &files("\"lib\"", "\"-DW=4\", \"-Ilib\""));
        let entry = configuration_entry(&built, &unit).expect("a file list has one");
        assert!(entry.starts_with("configuration: make blake3:"), "{entry}");
        // Resolved forms, and only the unit's own files' folders: a
        // cosmetic spelling or another file of the tool moves nothing.
        std::fs::write(root.join("lib/b.c"), "int b;\n").expect("b.c");
        for same in [
            files("\"lib/\"", "\"-DW=4\", \"-Ilib/\""),
            format!(
                "{}\n",
                files("\"lib\"", "\"-DW=4\", \"-Ilib\"").replace(
                    "include_dirs = [\"lib\"] }]",
                    "include_dirs = [\"lib\"] }, { path = \"lib/b.c\", include_dirs = [\"x\"] }]"
                )
            ),
        ] {
            assert_eq!(
                configuration_entry(&target(&root, &same), &unit).as_ref(),
                Some(&entry),
                "{same}"
            );
        }
        let ledger = Ledger::of(&built);
        let facts = crate::Facts::default();
        let store = |toolchain: Vec<String>| {
            let inputs = crate::verdict::VerdictInputs {
                toolchain,
                ..Default::default()
            };
            Verdict::new("u", inputs, Vec::new())
                .store(&ledger.verdict_latest_path("u"))
                .expect("stored");
        };
        let stale = |ctx: &TargetContext| -> Vec<String> {
            compute(ctx, &ledger, &facts, &unit, None)
                .expect("report")
                .verdict
                .stale
        };
        std::fs::create_dir_all(ledger.unit_dir("u")).expect("unit dir");
        store(vec!["cflags: -ffp-contract=off".into(), entry.clone()]);
        assert!(!stale(&built).contains(&"configuration".to_string()));
        for changed in [
            files("\"lib\"", "\"-DW=8\", \"-Ilib\""),
            files("\"lib\", \"inc\"", "\"-DW=4\", \"-Ilib\""),
            files("\"lib\"", "\"-DW=4\", \"-Iinc\""),
        ] {
            assert!(
                stale(&target(&root, &changed)).contains(&"configuration".to_string()),
                "{changed}"
            );
        }
        // A file-list verdict from before the entry.
        store(vec!["cflags: -ffp-contract=off".into()]);
        assert!(stale(&built).contains(&"configuration".to_string()));
        // The folder form: no entry recorded, none expected.
        let folder = target(
            &root,
            "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"lib\"",
        );
        assert_eq!(configuration_entry(&folder, &unit), None);
        assert!(!stale(&folder).contains(&"configuration".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The 2026-10-08 check's case: `inc/common.h` includes `"cfg.h"` and is
    /// reached from `a.c` (folders `inc, d1`) and `b.c` (folders `inc, d2`).
    /// The scan records no edge for that ambiguous name, but `a.c`'s
    /// compile reads `d1/cfg.h`: the planner's `source_hash`, `verify`'s
    /// `unit_source` (the same closure) and status hash it, so an edit to
    /// it stales the plan and the verdict.
    #[test]
    fn an_edit_behind_an_ambiguous_include_stales_the_verdict() {
        let root = std::env::temp_dir().join(format!(
            "ruharness-status-ambiguous-{}-{}",
            std::process::id(),
            crate::hash::random_hex(4)
        ));
        let put = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
            std::fs::write(p, text).expect("write");
        };
        put("a.c", "#include \"common.h\"\nint a(void) { return W; }\n");
        put("b.c", "#include \"common.h\"\nint b(void) { return W; }\n");
        put("inc/common.h", "#include \"cfg.h\"\n");
        put("d1/cfg.h", "#define W 1\n");
        put("d2/cfg.h", "#define W 2\n");
        let ctx = target(
            &root,
            "schema_version = 2\n[target]\nname = \"t\"\n\
             files = [{ path = \"a.c\", include_dirs = [\"inc\", \"d1\"] }, \
                      { path = \"b.c\", include_dirs = [\"inc\", \"d2\"] }]\n\
             configuration = { name = \"make\", from = \"stated\", flags = [] }",
        );
        // The facts as a scan records them: no edge for the ambiguous name.
        let record = |path: &str, includes: &[&str]| crate::facts::FileRecord {
            path: path.into(),
            hash: crate::hash::file_hash(&root.join(path)).expect("hash"),
            includes: includes.iter().map(|s| s.to_string()).collect(),
        };
        let symbol = |name: &str, file: &str| crate::facts::SymbolRecord {
            name: name.into(),
            kind: "function".into(),
            file: file.into(),
            visibility: "public".into(),
            signature: format!("int {name}(void)"),
            span: (2, 2),
        };
        let facts = crate::Facts {
            frontend: "c-tree-sitter".into(),
            files: vec![
                record("a.c", &["inc/common.h"]),
                record("b.c", &["inc/common.h"]),
                record("d1/cfg.h", &[]),
                record("d2/cfg.h", &[]),
                record("inc/common.h", &[]),
            ],
            symbols: vec![symbol("a", "a.c"), symbol("b", "b.c")],
            refs: Vec::new(),
        };
        let a = vec!["a.c".to_string()];
        assert_eq!(facts.include_closure(&a), ["a.c", "inc/common.h"]);
        assert_eq!(
            crate::sources::unit_closure(&ctx, &facts, &a),
            ["a.c", "d1/cfg.h", "inc/common.h"]
        );
        // The planner hashes the same closure (from the facts' hashes).
        let computed = crate::planner::compute_units_in(&ctx, &facts).expect("plan");
        let planned = computed.iter().find(|u| u.files == a).expect("a's unit");
        let mut moved = facts.clone();
        moved.files[2].hash = "blake3:other".into();
        let replanned = crate::planner::compute_units_in(&ctx, &moved).expect("plan");
        assert_ne!(
            replanned
                .iter()
                .find(|u| u.files == a)
                .expect("a")
                .source_hash,
            planned.source_hash,
            "d1/cfg.h is in a's source_hash"
        );
        let unit: Unit = toml::from_str(&format!(
            "id = \"u\"\nstatus = \"pending\"\nfiles = [\"a.c\"]\nsymbols = [\"a\"]\n\
             source_hash = \"{}\"\n",
            planned.source_hash
        ))
        .expect("unit");
        let ledger = Ledger::of(&ctx);
        std::fs::create_dir_all(ledger.unit_dir("u")).expect("unit dir");
        let closure = crate::sources::unit_closure(&ctx, &facts, &a);
        let inputs = crate::verdict::VerdictInputs {
            unit_source: crate::hash::file_set_hash_on_disk(&root, &closure).expect("hash"),
            toolchain: vec![configuration_entry(&ctx, &unit).expect("entry")],
            ..Default::default()
        };
        Verdict::new("u", inputs, Vec::new())
            .store(&ledger.verdict_latest_path("u"))
            .expect("stored");
        let now = || compute(&ctx, &ledger, &facts, &unit, None).expect("report");
        assert!(now().source_fresh);
        assert!(now().verdict.stale.is_empty(), "{:?}", now().verdict.stale);
        // b.c's header moves nothing of a's; a's moves both.
        put("d2/cfg.h", "#define W 3\n");
        assert!(now().verdict.stale.is_empty(), "{:?}", now().verdict.stale);
        put("d1/cfg.h", "#define W 4\n");
        assert!(!now().source_fresh);
        assert_eq!(now().verdict.stale, ["source"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn report(state: VerdictState, green: Option<bool>, stale: &[&str]) -> UnitReport {
        UnitReport {
            id: "u1".into(),
            status: "verified".into(),
            source_fresh: true,
            verdict: VerdictReport {
                state,
                green,
                stale: stale.iter().map(|s| s.to_string()).collect(),
            },
            contradiction: false,
            write_in_flight: None,
            promotion_interrupted: None,
            attempts: Vec::new(),
            features: None,
            made_elsewhere: false,
        }
    }

    #[test]
    fn the_human_line_keeps_its_bytes() {
        let r = report(VerdictState::Present, Some(true), &[]);
        assert_eq!(
            r.render_line(),
            "status: u1 [verified] plan=fresh verdict=green (fresh)"
        );
        let mut r = report(
            VerdictState::Present,
            Some(false),
            &["rust-crate", "driver"],
        );
        r.source_fresh = false;
        r.contradiction = true;
        assert_eq!(
            r.render_line(),
            "status: u1 [verified] plan=SOURCE-STALE verdict=red (STALE: rust-crate, driver)  \
             << CONTRADICTION: status and verdict evidence disagree"
        );
        r.write_in_flight = Some(Holder {
            pid: 7,
            command: "verify u1".into(),
            started: "t".into(),
        });
        assert!(r
            .render_line()
            .contains("write in flight (pid 7, `verify u1`)"));
        assert!(!r.render_line().contains("CONTRADICTION"));
        r.write_in_flight = None;
        r.promotion_interrupted = Some("a-0123456789ab".into());
        assert!(r.render_line().ends_with(
            "<< promotion of a-0123456789ab interrupted — the next writing command recovers it"
        ));
        assert!(!r.render_line().contains("CONTRADICTION"));
        assert_eq!(
            report(VerdictState::Missing, None, &[]).render_line(),
            "status: u1 [verified] plan=fresh verdict=no verdict"
        );
        assert_eq!(
            report(VerdictState::Unreadable, None, &[]).render_line(),
            "status: u1 [verified] plan=fresh verdict=verdict UNREADABLE (corrupt?)"
        );
    }

    #[test]
    fn the_event_shape_is_pinned() {
        let mut r = report(VerdictState::Present, Some(true), &["source"]);
        r.attempts.push(AttemptSummary {
            id: "a-1".into(),
            provider_kind: "external".into(),
            outcome: "green".into(),
            bound: false,
        });
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(
            json,
            "{\"id\":\"u1\",\"status\":\"verified\",\"source_fresh\":true,\"verdict\":{\"state\":\
             \"present\",\"green\":true,\"stale\":[\"source\"]},\"contradiction\":false,\
             \"attempts\":[{\"id\":\"a-1\",\"provider_kind\":\"external\",\"outcome\":\"green\",\
             \"bound\":false}]}"
        );
        assert_eq!(
            r.render_attempts_line().unwrap(),
            "status:   attempts: 1 (0 bound to current source) [a-1:external:green]"
        );
        let missing = report(VerdictState::Missing, None, &[]);
        assert!(serde_json::to_string(&missing)
            .unwrap()
            .contains("\"verdict\":{\"state\":\"missing\",\"stale\":[]}"));
    }

    /// A verdict made elsewhere is marked on the line and in the event,
    /// and stays out of freshness and contradiction.
    #[test]
    fn a_verdict_made_elsewhere_is_marked() {
        let mut r = report(VerdictState::Present, Some(true), &[]);
        r.made_elsewhere = true;
        assert!(r.fresh_green());
        assert_eq!(
            r.render_line(),
            "status: u1 [verified] plan=fresh verdict=green (fresh) made-elsewhere"
        );
        assert!(serde_json::to_string(&r)
            .unwrap()
            .ends_with(",\"made_elsewhere\":true}"));
        r.made_elsewhere = false;
        assert!(!serde_json::to_string(&r)
            .unwrap()
            .contains("made_elsewhere"));
    }

    #[test]
    fn feature_coverage_is_a_marker_beside_the_verdict() {
        use crate::features::Coverage;
        let mut r = report(VerdictState::Present, Some(true), &[]);
        r.features = Some(Coverage::Behind(vec!["changed".into(), "program".into()]));
        assert!(r.fresh_green(), "coverage never enters freshness");
        assert_eq!(
            r.render_line(),
            "status: u1 [verified] plan=fresh verdict=green (fresh) features=behind(changed, program)"
        );
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            json.ends_with(",\"features\":[\"changed\",\"program\"]}"),
            "{json}"
        );
        r.features = Some(Coverage::Current);
        assert!(serde_json::to_string(&r)
            .unwrap()
            .ends_with(",\"features\":\"current\"}"));
        assert!(r
            .render_line()
            .ends_with("verdict=green (fresh) features=current"));
    }
}
