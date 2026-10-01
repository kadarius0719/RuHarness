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

/// About how long a row at 31 runs takes (§6): 3 + 62 runs at the C's
/// median clock time plus a tenth of a second.
fn seconds_a_row(row: &Row) -> u64 {
    let clock = row
        .c
        .as_deref()
        .and_then(|runs| {
            let v: Vec<Option<f64>> = runs.iter().map(|r| r.wall_us.map(|w| w as f64)).collect();
            harness_core::perf::stats::median(&v)
        })
        .unwrap_or(1e6)
        / 1e6;
    ((3.0 + 62.0) * (clock + 0.1)).ceil() as u64
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
            words::more_runs_words(&more_runs_command(side, &row.workload), seconds_a_row(row))
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

/// `harness perf show` (§3.9): every stored row's words, rebuilt, with why
/// it is out of date; the computer checked only when the launcher cache is
/// current, the compilers only as allowlisted tool runs; `--no-check`
/// skips both.
pub(crate) fn cmd_show(target: PathBuf, no_check: bool) -> Result<u8> {
    let ctx = TargetContext::load(&target)?;
    let ledger = Ledger::new(&ctx.root);
    let dir = perf_dir(&ctx)?;
    let program = res::read_program(&res::program_path(&dir))?;
    let workloads = match wl::load(&ctx.root)? {
        WorkloadsState::Ready(w) => Some(w),
        state => {
            out(format!("perf: {}", state.blocker().unwrap_or_default()));
            None
        }
    };
    let facts = Facts::load(&ledger.facts_path()).ok();
    let plan = plan(&ledger)?;
    let program_digest = facts
        .as_ref()
        .map(|f| harness_core::features::program_digest_now(&ctx, f))
        .unwrap_or_default();
    let name = harness_core::features::program_name(&ctx.config);
    let measurable = facts
        .as_ref()
        .and_then(|f| harness_oracle::perf_measurable(&ctx, &plan, f).ok());
    let computer = if no_check {
        None
    } else {
        harness_oracle::perf_computer_if_cached()
    };
    let compilers = if no_check { None } else { compilers(&ctx) };
    if !no_check && computer.is_none() {
        out("perf: computer not checked — run harness perf run once".into());
    }
    if !no_check && compilers.is_none() {
        out("perf: compilers not checked".into());
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
            let today = harness_core::perf::currency::Today {
                workload: today_workload.as_deref(),
                program: &program_digest,
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
    let units_dir = dir.join(res::UNITS_DIR);
    if let Ok(entries) = std::fs::read_dir(&units_dir) {
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
        for id in ids {
            let Some(file) = res::read_unit(&res::unit_path(&dir, &id), &id)? else {
                continue;
            };
            let replaces = plan
                .units
                .iter()
                .find(|u| u.id == id)
                .map(|u| u.oracle_param_list("replaces"));
            if replaces.is_none() {
                out(format!("perf: {id} — no longer in the plan"));
                continue;
            }
            show(&file.rows, Side::Unit(&id), RowKind::Unit, &id, replaces);
        }
    }
    if printed == 0 {
        out("perf: nothing measured yet — run harness perf run".into());
    }
    Ok(0)
}

/// `cc --version` and `rustc -V`'s first lines, as tool runs, when the
/// target's allowlist has them.
fn compilers(ctx: &TargetContext) -> Option<(String, String)> {
    let allow: Vec<&str> = ctx
        .config
        .oracle
        .get("allowlist")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|t| t.as_str()).collect())
        .unwrap_or_default();
    if !allow.contains(&"cc") || !allow.contains(&"rustc") {
        return None;
    }
    let first = |cmd: &str, arg: &str| -> Option<String> {
        let out = std::process::Command::new(cmd)
            .arg(arg)
            .current_dir(&ctx.root)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        Some(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()?
                .trim()
                .chars()
                .take(160)
                .collect(),
        )
    };
    Some((first("cc", "--version")?, first("rustc", "-V")?))
}
