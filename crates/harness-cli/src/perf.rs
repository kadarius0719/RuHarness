//! `harness perf run | init | save | show` (docs/PERF-DESIGN.md §3.10):
//! information only — perf writes no verdict and no plan status, and
//! `verify`, `migrate`, `promote` and `bench check` never read it.

use crate::{lock_ledger, out, report, safe_ledger_dir};
use anyhow::{bail, Context, Result};
use harness_core::ledger::Ledger;
use harness_core::perf::results::{self as res, Row, RowKind};
use harness_core::perf::words::{self as words, Side};
use harness_core::perf::workloads::{self as wl, Workloads, WorkloadsState};
use harness_core::{Facts, Plan, TargetContext};
use std::path::{Path, PathBuf};

/// `migration/perf/`, resolved with links refused.
fn perf_dir(ctx: &TargetContext) -> Result<PathBuf> {
    safe_ledger_dir(
        &ctx.root,
        &[
            harness_core::ledger::MIGRATION_DIR,
            harness_core::perf::PERF_DIR,
        ],
    )
}

fn shown(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// `harness perf init`: the starter, never over an existing file.
pub(crate) fn cmd_init(target: PathBuf) -> Result<u8> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, "perf init")?;
    let dir = perf_dir(&ctx)?;
    let path = dir.join(wl::WORKLOADS_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => bail!(
            "{} exists; `perf init` never overwrites it",
            shown(&ctx.root, &path)
        ),
        Err(e) => return Err(e).with_context(|| format!("inspecting {}", path.display())),
    }
    harness_core::ledger::write_atomic(&path, wl::STARTER.as_bytes())?;
    out(format!(
        "perf: wrote a starter to {} — add a [[workload]], then run `harness perf run`",
        shown(&ctx.root, &path)
    ));
    Ok(0)
}

/// `harness perf save`: the new text on stdin, `--bytes` long, saved only
/// when it validates and the file on disk is still the one `--expect` names
/// (its blake3, or `none`). The input's existence and size are checked when
/// perf reads it (§3.1).
pub(crate) fn cmd_save(target: PathBuf, expect: String, bytes: u64) -> Result<u8> {
    use std::io::{IsTerminal, Read};
    if bytes > wl::MAX_WORKLOADS_BYTES {
        bail!(
            "--bytes {bytes} is more than the {} a workloads file may hold",
            wl::MAX_WORKLOADS_BYTES
        );
    }
    if std::io::stdin().is_terminal() {
        bail!("stdin is a terminal: pipe the new workloads file in");
    }
    let mut text = Vec::new();
    std::io::stdin()
        .lock()
        .take(bytes + 1)
        .read_to_end(&mut text)
        .context("reading the new workloads file from stdin")?;
    if text.len() as u64 != bytes {
        bail!(
            "{} bytes on stdin, not the {bytes} --bytes names: cut short or changed, not saved",
            text.len()
        );
    }
    let text = String::from_utf8(text).context("the workloads file must be UTF-8")?;
    if expect != "none" && !expect.starts_with(harness_core::hash::HASH_PREFIX) {
        bail!("--expect takes the blake3 of the file's current bytes, or `none`");
    }
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, "perf save")?;
    let dir = perf_dir(&ctx)?;
    let path = dir.join(wl::WORKLOADS_FILE);
    let current = match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "none".to_string(),
        Ok(m) if m.file_type().is_file() => harness_core::hash::bytes_hash(
            &harness_core::ledger::read_regular(&path, wl::MAX_WORKLOADS_BYTES + 1)?,
        ),
        Ok(_) => bail!(
            "{} is not a regular file; replace it with one outside the harness",
            shown(&ctx.root, &path)
        ),
        Err(e) => return Err(e).with_context(|| format!("inspecting {}", path.display())),
    };
    if current != expect {
        bail!(
            "{} changed since the edit started; nothing was saved",
            shown(&ctx.root, &path)
        );
    }
    match wl::parse(&text, &path) {
        Ok(_) => {}
        Err(wl::ParseError::Rule(e)) => bail!("{e}"),
        Err(wl::ParseError::TooNew(e)) => return Err(e.into()),
    }
    harness_core::ledger::write_atomic(&path, text.as_bytes())?;
    out(format!("perf: saved {}", shown(&ctx.root, &path)));
    Ok(0)
}

/// The workloads, or the state's words as an error (exit 1, §3.1 States).
fn workloads(ctx: &TargetContext) -> Result<Workloads> {
    match wl::load(&ctx.root)? {
        WorkloadsState::Ready(w) => Ok(w),
        state => bail!("{}", state.blocker().unwrap_or_default()),
    }
}

/// The plan, or an empty one when there is none yet (the C alone can still
/// be measured on day one, §3.6).
fn plan(ledger: &Ledger) -> Result<Plan> {
    let path = ledger.plan_path();
    if !path.exists() {
        return Ok(Plan {
            schema_version: 1,
            target: String::new(),
            units: Vec::new(),
        });
    }
    Ok(Plan::load(&path)?)
}

fn fresh_facts(ctx: &TargetContext, ledger: &Ledger) -> Result<Facts> {
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    if crate::stale_fact_files(ctx, &facts) > 0
        || harness_core::features::program_digest_now(ctx, &facts)
            == harness_core::features::STALE_PROGRAM
    {
        bail!("the program's C changed since the scan: scan the project first, then measure");
    }
    Ok(facts)
}

/// `--as-it-stands-only` with fewer than two measurable units, refused
/// before the launcher, the C or any crate is built (§3.10: "says why"):
/// the measurable units, and each verified unit left out with why, as the
/// run's own line names them ("u-tree left out: verify it first"; ten
/// named, the rest counted). A plan over perf's size is refused by its size
/// first, as the run refuses it.
fn as_it_stands_needs_two(ctx: &TargetContext, plan: &Plan, facts: &Facts) -> Result<()> {
    match harness_oracle::perf_selection(ctx, plan, facts)?.needs_two() {
        Some(words) => bail!("{words}"),
        None => Ok(()),
    }
}

/// `harness perf run` (§3.10).
pub(crate) fn cmd_run(
    target: PathBuf,
    units: Vec<String>,
    workload_ids: Vec<String>,
    runs: Option<u32>,
    as_it_stands_only: bool,
) -> Result<u8> {
    if !cfg!(target_os = "macos") {
        bail!("perf runs on macOS only for now — the Linux launcher is not built yet");
    }
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let _lock = lock_ledger(&ledger, harness_core::perf::PERF_RUN_LOCK)?;
    let workloads = workloads(&ctx)?;
    let facts = fresh_facts(&ctx, &ledger)?;
    let plan = plan(&ledger)?;
    if as_it_stands_only {
        as_it_stands_needs_two(&ctx, &plan, &facts)?;
    }
    let dir = perf_dir(&ctx)?;
    let request = harness_oracle::PerfRequest {
        units,
        workloads: workload_ids,
        runs,
        as_it_stands_only,
    };
    let mut progress = Progress {
        workloads: &workloads,
        root: ctx.root.clone(),
    };
    let summary = harness_oracle::perf_run(
        &ctx,
        &plan,
        &facts,
        &workloads,
        &dir,
        &request,
        &mut progress,
    )?;
    let rows = |n: usize| if n == 1 { "row" } else { "rows" };
    out(format!(
        "perf: measured {} {}, {} too short, {} behave{} differently — wrote {} (commit it to keep \
         a history; perf compares what the program prints and how it ends)",
        summary.measured,
        rows(summary.measured),
        summary.too_short,
        summary.behaves_differently,
        if summary.behaves_differently == 1 { "s" } else { "" },
        shown(&ctx.root, &dir)
    ));
    Ok(0)
}

/// A row's label: `the C`, `the program as it stands`, or the unit's id.
fn label(side: harness_oracle::RowSide<'_>) -> (String, Side<'_>, RowKind) {
    match side {
        harness_oracle::RowSide::C => ("the C".into(), Side::C, RowKind::CAlone),
        harness_oracle::RowSide::Program => (
            "the program as it stands".into(),
            Side::AsItStands,
            RowKind::AsItStands,
        ),
        harness_oracle::RowSide::Unit(id) => (id.to_string(), Side::Unit(id), RowKind::Unit),
    }
}

/// The "measure again with 31 runs" command for a row's side (§3.8).
fn more_runs_command(side: Side<'_>, workload: &str) -> String {
    match side {
        Side::Unit(id) => format!("harness perf run --unit {id} --workload {workload} --runs 31"),
        _ => format!("harness perf run --as-it-stands-only --workload {workload} --runs 31"),
    }
}

/// About how long the "31 runs" command for a row's side takes (§6): its
/// 65 runs at the C's clock time, the first execs, the C's compile and
/// link, the crates built and the row's link. The words follow a run (or
/// `perf show`), so the launcher counts as built.
fn seconds_a_row(row: &Row, side: Side<'_>) -> u64 {
    use harness_core::perf::estimate::{c_clock, Estimate, Job};
    let crates = match side {
        Side::AsItStands => row.inputs.units.as_ref().map_or(0, Vec::len) as u32,
        _ => 1,
    };
    let job = Job {
        workloads: vec![(31, Some(c_clock(row).unwrap_or(1.0)))],
        c_alone: false,
        rows: 1,
        crates,
        links: 1,
    };
    match job.estimate() {
        Estimate::Seconds(s) => s,
        Estimate::Runs { .. } => 0,
    }
}

/// The words of one row, with the 31-run offer when it applies.
fn row_words(row: &Row, side: Side<'_>, input: Option<&str>) -> words::RowWords {
    let mut w = words::words(
        row,
        &words::Context {
            side,
            workload: &row.workload,
            input,
        },
    );
    if w.offers_more_runs {
        w.headline = format!(
            "{} {}",
            w.headline,
            words::more_runs_words(
                &more_runs_command(side, &row.workload),
                seconds_a_row(row, side)
            )
        );
    }
    w
}

/// The progress of a run: human lines and `perf-row` events (§3.10).
struct Progress<'a> {
    workloads: &'a Workloads,
    root: PathBuf,
}

impl harness_oracle::PerfProgress for Progress<'_> {
    fn message(&mut self, text: &str) {
        out(format!("perf: {text}"));
    }

    fn row(&mut self, side: harness_oracle::RowSide<'_>, row: &Row) {
        let (label, words_side, _) = label(side);
        let input = self
            .workloads
            .get(&row.workload)
            .and_then(|w| w.input.as_deref());
        let mut w = row_words(row, words_side, input);
        if row.outcome == "behaves-differently"
            && row
                .first_difference
                .as_ref()
                .is_some_and(|d| !d.kept.is_empty())
        {
            let folder = match side {
                harness_oracle::RowSide::Unit(id) => {
                    format!("migration/build/.perf-out/units/{id}/")
                }
                _ => "migration/build/.perf-out/program/".to_string(),
            };
            w.details.push(format!("both outputs are kept in {folder}"));
        }
        let _ = &self.root;
        for line in words::cli_lines(&label, &row.workload, &w) {
            out(line);
        }
        #[derive(serde::Serialize)]
        struct RowEvent<'a> {
            k: &'static str,
            side: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            unit: Option<&'a str>,
            workload: &'a str,
            outcome: &'a str,
            words: &'a str,
        }
        let (side_name, unit) = match side {
            harness_oracle::RowSide::C => ("c", None),
            harness_oracle::RowSide::Program => ("program", None),
            harness_oracle::RowSide::Unit(id) => ("unit", Some(id)),
        };
        report::event(&RowEvent {
            k: "perf-row",
            side: side_name,
            unit,
            workload: &row.workload,
            outcome: &row.outcome,
            words: &w.headline,
        });
    }
}

/// A folder `perf show` reads, every part checked with lstat: `None` when
/// it is not there, refused when a part is a link or not a folder (§3.9:
/// links are refused on read too). It creates nothing.
fn stored_dir(root: &Path, parts: &[&str]) -> Result<Option<PathBuf>> {
    let mut cur = root.to_path_buf();
    for part in parts {
        cur = cur.join(part);
        match std::fs::symlink_metadata(&cur) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => bail!(
                "{}: must be a directory (a link is refused)",
                shown(root, &cur)
            ),
            Err(e) => return Err(e).with_context(|| format!("inspecting {}", cur.display())),
        }
    }
    Ok(Some(cur))
}

/// One unit's stored rows as `perf show` reads them.
struct StoredUnit {
    id: String,
    rows: Vec<Row>,
    /// The unit's `replaces` today; `None` when it is no longer in the plan
    /// (its rows are then not shown).
    replaces: Option<Vec<String>>,
}

/// Every unit's results file in `units_dir` (checked by [`stored_dir`]), in
/// id order; a name that is not a clean unit id is not read. A unit still
/// in the plan has its file read strictly; one no longer in the plan is
/// named without being read, as the cockpit does. A file (or the folder)
/// that cannot be read is kept as its error, beside the units that read:
/// one bad file hides no other row (the cockpit shows the rest too), and
/// `perf show` reports it after the rows.
fn stored_units(
    root: &Path,
    units_dir: &Path,
    plan: &Plan,
) -> (Vec<StoredUnit>, Vec<anyhow::Error>) {
    let entries = match std::fs::read_dir(units_dir) {
        Ok(entries) => entries,
        Err(e) => {
            return (
                Vec::new(),
                vec![anyhow::anyhow!("{}: {e}", shown(root, units_dir))],
            )
        }
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".json"))
                .map(str::to_string)
        })
        .filter(|id| harness_core::plan::is_clean_segment(id))
        .collect();
    ids.sort();
    let mut units = Vec::new();
    let mut bad = Vec::new();
    for id in ids {
        let Some(unit) = plan.units.iter().find(|u| u.id == id) else {
            units.push(StoredUnit {
                id,
                rows: Vec::new(),
                replaces: None,
            });
            continue;
        };
        let file = match res::read_unit(&units_dir.join(format!("{id}.json")), &id) {
            Ok(Some(file)) => file,
            Ok(None) => continue,
            Err(e) => {
                bad.push(anyhow::Error::from(e));
                continue;
            }
        };
        units.push(StoredUnit {
            id,
            rows: file.rows,
            replaces: Some(unit.oracle_param_list("replaces")),
        });
    }
    (units, bad)
}

/// `harness perf show` (§3.9): every stored row's words, rebuilt, with why
/// it is out of date; the computer checked only when the launcher cache is
/// current, the compilers only as tool runs (sandboxed; without a sandbox
/// only with `--allow-unsandboxed`); `--no-check` skips both, and with no
/// row stored there is nothing to judge, so neither is checked or said —
/// "nothing measured yet" is the line. Without facts the C cannot be
/// hashed: it is not judged (said once), never "the C changed" on every
/// row; nor are the units the program as it stands holds, said in the same
/// line when such a row is stored. A results file that cannot be read (the
/// program's or a unit's), or a units folder that is not one, hides no
/// other row: the rows that read are shown, then every error is named and
/// the show exits 1. It writes nothing.
pub(crate) fn cmd_show(target: PathBuf, no_check: bool, allow_unsandboxed: bool) -> Result<u8> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let perf_parts = [
        harness_core::ledger::MIGRATION_DIR,
        harness_core::perf::PERF_DIR,
    ];
    let dir = stored_dir(&ctx.root, &perf_parts)?;
    // What cannot be read is named after the rows that read (exit 1): the
    // program's file, the units' folder, each unit's file.
    let mut bad: Vec<anyhow::Error> = Vec::new();
    let program = match &dir {
        Some(dir) => res::read_program(&res::program_path(dir)).unwrap_or_else(|e| {
            bad.push(e.into());
            None
        }),
        None => None,
    };
    // The units' folder is checked the same way: a linked one would show
    // another folder's files as this target's rows.
    let units_dir = match &dir {
        Some(_) => stored_dir(&ctx.root, &[perf_parts[0], perf_parts[1], res::UNITS_DIR])
            .unwrap_or_else(|e| {
                bad.push(e);
                None
            }),
        None => None,
    };
    let workloads = match wl::load(&ctx.root)? {
        WorkloadsState::Ready(w) => Some(w),
        state => {
            out(format!("perf: {}", state.blocker().unwrap_or_default()));
            None
        }
    };
    let facts = Facts::load(&ledger.facts_path()).ok();
    let plan = plan(&ledger)?;
    let units = match &units_dir {
        Some(units_dir) => {
            let (units, bad_units) = stored_units(&ctx.root, units_dir, &plan);
            bad.extend(bad_units);
            units
        }
        None => Vec::new(),
    };
    // The rows to judge (a unit no longer in the plan shows none): with
    // none, no check is worth its runs (§3.9).
    let to_judge = program
        .as_ref()
        .map_or(0, |p| p.c_alone.len() + p.as_it_stands.len())
        + units
            .iter()
            .filter(|u| u.replaces.is_some())
            .map(|u| u.rows.len())
            .sum::<usize>();
    let check = !no_check && to_judge > 0;
    // `None`: no facts (none, or unreadable), so the C cannot be hashed.
    let program_digest = facts
        .as_ref()
        .map(|f| harness_core::features::program_digest_now(&ctx, f));
    let name = harness_core::features::program_name(&ctx.config);
    // `None` (no facts, or the units could not be read): which units the
    // program as it stands holds today is not judged — said below.
    let measurable = facts
        .as_ref()
        .map(|f| harness_oracle::perf_measurable_for_currency(&ctx, &plan, f));
    let (measurable, measurable_error) = match measurable {
        Some(Ok(ids)) => (Some(ids), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let as_it_stands_stored = program.as_ref().is_some_and(|p| !p.as_it_stands.is_empty());
    let computer = if check {
        harness_oracle::perf_computer_if_cached()
    } else {
        None
    };
    let compilers = if check {
        harness_oracle::perf_compilers(&ctx, allow_unsandboxed)
    } else {
        None
    };
    if check && computer.is_none() {
        out("perf: computer not checked — run harness perf run once".into());
    }
    if check && compilers.is_none() {
        out("perf: compilers not checked".into());
    }
    // Said once for the show. Without facts the units the program as it
    // stands holds cannot be checked either ("left out now", "accepted
    // since", the plan's order): a row of it would read current unjudged.
    if to_judge > 0 && program_digest.is_none() {
        out(if as_it_stands_stored {
            "perf: the C and the units the program as it stands holds not checked: no facts \
             — run harness scan"
                .into()
        } else {
            "perf: the C not checked: no facts — run harness scan".into()
        });
    } else if let (true, Some(e)) = (as_it_stands_stored, &measurable_error) {
        out(format!(
            "perf: the units the program as it stands holds not checked: {e}"
        ));
    }
    let crate_digest = |id: &str| -> Option<String> {
        let unit = plan.units.iter().find(|u| u.id == id)?;
        let krate = unit.oracle_param_str("rust_crate")?;
        harness_core::hash::unit_crate_file_set_hash(&ctx.root, &ledger.unit_dir(id).join(krate))
            .ok()
    };
    let workload_digest = |id: &str| -> Option<String> {
        let w = workloads.as_ref()?.get(id)?;
        let bytes = match &w.input {
            Some(rel) => Some(wl::read_input(&ctx.root, rel).ok()?),
            None => None,
        };
        Some(wl::digest(w, bytes.as_deref()))
    };
    let mut printed = 0;
    let mut show = |rows: &[Row],
                    side: Side<'_>,
                    kind: RowKind,
                    label: &str,
                    replaces: Option<Vec<String>>| {
        for row in rows {
            let input = workloads
                .as_ref()
                .and_then(|w| w.get(&row.workload))
                .and_then(|w| w.input.as_deref());
            let mut w = row_words(row, side, input);
            let today_workload = workload_digest(&row.workload);
            // Without facts only the C's comparison is skipped (said once
            // above), as the cockpit does: the row is held to its own
            // program digest, and the rest is still judged.
            let program_today = program_digest.as_deref().unwrap_or(&row.inputs.program);
            let today = harness_core::perf::currency::Today {
                workload: today_workload.as_deref(),
                program: program_today,
                crate_digest: &crate_digest,
                replaces: replaces.as_deref(),
                program_name: &name,
                measurable: measurable.as_deref(),
                computer: computer.as_ref(),
                compilers: compilers.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
            };
            let why = harness_core::perf::currency::out_of_date(row, kind, &today);
            if !why.is_empty() {
                w.details.push(format!("out of date: {}", why.join("; ")));
            }
            for line in words::cli_lines(label, &row.workload, &w) {
                out(line);
            }
            printed += 1;
        }
    };
    if let Some(p) = &program {
        show(&p.c_alone, Side::C, RowKind::CAlone, "the C", None);
        show(
            &p.as_it_stands,
            Side::AsItStands,
            RowKind::AsItStands,
            "the program as it stands",
            None,
        );
    }
    for unit in &units {
        match &unit.replaces {
            Some(replaces) => show(
                &unit.rows,
                Side::Unit(&unit.id),
                RowKind::Unit,
                &unit.id,
                Some(replaces.clone()),
            ),
            None => out(format!("perf: {} — no longer in the plan", unit.id)),
        }
    }
    // The rows that read are shown; then each thing that did not read is
    // named, one a line, and the show fails (exit 1).
    let mut bad = bad.into_iter();
    if let Some(first) = bad.next() {
        let rest: Vec<String> = bad.map(|e| format!("{e:#}")).collect();
        if rest.is_empty() {
            return Err(first);
        }
        bail!("{first:#}\n{}", rest.join("\n"));
    }
    if printed == 0 {
        out("perf: nothing measured yet — run harness perf run".into());
    }
    Ok(0)
}
