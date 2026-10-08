//! The `harness` binary (docs/SCHEMAS.md "CLI contract").
//!
//! Exit codes: 0 ok/green · 1 harness error (including stale refusals) ·
//! 2 usage error (clap's own) · 10 oracle red.

#![forbid(unsafe_code)]

mod bench;
mod features;
mod gen_driver;
mod hand_edit;
mod perf;
mod project;
mod promote;
mod report;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use harness_core::adopt;
use harness_core::attempts;
use harness_core::ledger::Ledger;
use harness_core::ledger::WriterLock;
use harness_core::traits::OracleStrategy;
use harness_core::{hash, plan, planner, Error, Facts, Plan, TargetContext, UnitStatus};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Exit code for a red oracle verdict (distinct from clap's usage code 2).
pub(crate) const EXIT_ORACLE_RED: u8 = 10;

#[derive(Parser)]
#[command(
    name = "harness",
    version,
    about = "Incremental, verifiable migration to Rust"
)]
struct Cli {
    /// Machine-readable events on stdout (newline-delimited JSON,
    /// `ruharness-events`); human logs stay on stderr
    #[arg(long, global = true)]
    json: bool,
    /// Trust the migration results already in this folder (made on another
    /// computer, or shipped in a download) from now on, on this computer:
    /// deletes their build folders; their verdicts stay claims until
    /// `harness verify` runs them here. Needed once per folder
    #[arg(long, global = true)]
    adopt: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

/// `--target` and `--tool`, on every subcommand that opens a target
/// (docs/PROJECT-MAP-DESIGN.md §3.7 "Finding a target").
#[derive(clap::Args, Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetArg {
    /// Target repository root: a folder with a harness.toml, or a project
    /// whose mapped tools live under migration/tools/
    #[arg(long, default_value = ".")]
    pub(crate) target: PathBuf,
    /// The mapped tool to open (its id, as `harness project map` prints it):
    /// loads migration/tools/<ID>/harness.toml, never the root's. Without
    /// it: the root's harness.toml, else the project's only tool
    #[arg(long, value_name = "ID", value_parser = parse_tool)]
    pub(crate) tool: Option<String>,
}

/// `--tool`'s value: a tool id, else the one-sentence refusal (exit 2).
fn parse_tool(id: &str) -> std::result::Result<String, String> {
    harness_core::config::check_tool_id(id).map(|()| id.to_string())
}

impl TargetArg {
    /// Load the target by the lookup order.
    pub(crate) fn load(&self) -> Result<TargetContext> {
        Ok(TargetContext::open(&self.target, self.tool.as_deref())?)
    }

    /// The flags that name this target again in a resume command line,
    /// each value shell-quoted and attached.
    pub(crate) fn resume_args(&self) -> String {
        let q = report::shell_quote;
        let mut args = format!(" --target={}", q(&self.target.to_string_lossy()));
        if let Some(tool) = &self.tool {
            args.push_str(&format!(" --tool={}", q(tool)));
        }
        args
    }
}

impl Cmd {
    /// The folder whose ledger(s) this command opens, and how it is adopted
    /// (docs/PROJECT-MAP-DESIGN.md §3.7): a target is one project; a bench
    /// suite is one root covering every case under it.
    fn ledger_root(&self) -> Option<(PathBuf, adopt::Scope)> {
        let project = |t: &PathBuf| Some((t.clone(), adopt::Scope::Project));
        match self {
            Cmd::Scan { target }
            | Cmd::Plan { target }
            | Cmd::Verify { target, .. }
            | Cmd::State {
                cmd: StateCmd::Status { target },
            }
            | Cmd::Detect { target }
            | Cmd::Observe { target }
            | Cmd::Review { target, .. }
            | Cmd::Migrate { target, .. }
            | Cmd::Override { target, .. }
            | Cmd::Promote { target, .. }
            | Cmd::GenDriver { target, .. }
            | Cmd::SyncRuntime { target, .. } => project(&target.target),
            Cmd::Features { cmd } => match cmd {
                FeaturesCmd::Init { target }
                | FeaturesCmd::Save { target, .. }
                | FeaturesCmd::Map { target, .. } => project(&target.target),
            },
            Cmd::Project {
                cmd: ProjectCmd::Map { target, .. },
            } => project(target),
            Cmd::Perf { cmd } => match cmd {
                PerfCmd::Run { target, .. }
                | PerfCmd::Init { target }
                | PerfCmd::Save { target, .. }
                | PerfCmd::Show { target, .. } => project(&target.target),
            },
            Cmd::Bench { cmd } => bench::suite_root(cmd).map(|s| (s, adopt::Scope::Suite)),
        }
    }
}

/// Before a command opens its ledger: adopt it when `--adopt` was given
/// (and say what that did), refuse a bench suite made elsewhere up front
/// (a target is refused by `TargetContext::load`), and note a folder that
/// holds no ledger yet — what this command writes there is its own. `true`
/// when the folder held no ledger.
fn open_ledger(root: &Path, scope: adopt::Scope, adopt_it: bool) -> Result<bool> {
    if !root.is_dir() {
        // The command says what is wrong with its folder.
        return Ok(false);
    }
    let had = match scope {
        adopt::Scope::Project => adopt::has_ledger(root),
        adopt::Scope::Suite => adopt::suite_has_ledger(root),
    };
    if adopt_it {
        let done = match scope {
            adopt::Scope::Project => adopt::adopt(root)?,
            adopt::Scope::Suite => adopt::adopt_suite(root)?,
        };
        for line in done.describe() {
            out(line);
        }
    } else if scope == adopt::Scope::Suite {
        adopt::check_suite(root)?;
    }
    if !had {
        adopt::note_created_here(root);
    }
    Ok(!had)
}

#[derive(Subcommand)]
enum Cmd {
    /// Scan the target and regenerate migration/facts.jsonl
    Scan {
        #[command(flatten)]
        target: TargetArg,
    },
    /// Reconcile migration/plan.toml against the facts
    Plan {
        #[command(flatten)]
        target: TargetArg,
    },
    /// Run a unit's oracle and record the verdict
    Verify {
        /// Unit id from plan.toml
        unit: String,
        #[command(flatten)]
        target: TargetArg,
        /// Run target/model-derived code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
    },
    /// Ledger state queries
    State {
        #[command(subcommand)]
        cmd: StateCmd,
    },
    /// The person's features: scenarios checked on every verify, and the map
    /// of which functions each runs (docs/FEATURES-DESIGN.md)
    Features {
        #[command(subcommand)]
        cmd: FeaturesCmd,
    },
    /// A whole C project: which files make up its programs
    /// (docs/PROJECT-MAP-DESIGN.md)
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// The C against the Rust in use, on the person's workloads: CPU time,
    /// instructions, memory (docs/PERF-DESIGN.md) — information only
    Perf {
        #[command(subcommand)]
        cmd: PerfCmd,
    },
    /// Run the hazard detectors and regenerate observer findings
    Detect {
        #[command(flatten)]
        target: TargetArg,
    },
    /// Triage findings (LLM pass) and render observations.md
    Observe {
        #[command(flatten)]
        target: TargetArg,
    },
    /// Record a human review of a triaged finding
    Review {
        /// Finding id (f-...)
        finding: String,
        /// Uphold the model's dismissal
        #[arg(long, conflicts_with = "reinstate")]
        uphold_dismiss: bool,
        /// Reinstate a dismissed finding
        #[arg(long)]
        reinstate: bool,
        /// Optional note
        #[arg(long, default_value = "")]
        note: String,
        #[command(flatten)]
        target: TargetArg,
    },
    /// Translate a unit through the configured LLM provider and verify it
    Migrate {
        /// Unit id from plan.toml
        unit: String,
        #[command(flatten)]
        target: TargetArg,
        /// Provider profile override (built-in or user-level profile name)
        #[arg(long)]
        provider: Option<String>,
        /// Model override
        #[arg(long)]
        model: Option<String>,
        /// Promote a green candidate even when the unit is already verified
        #[arg(long, conflicts_with = "no_promote")]
        promote: bool,
        /// Record a green attempt without promoting it (`harness promote`
        /// does that later, explicitly)
        #[arg(long)]
        no_promote: bool,
        /// Run target/model-derived code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
        /// Record a NEW sample when this live attempt already finished
        #[arg(long)]
        retry: bool,
        /// Pin the recorded attempt a `--provider replay` run verifies
        #[arg(long)]
        attempt: Option<String>,
        /// Pose a STEER attempt: the reviewer's note over the finished
        /// attempt named by --from (both are required together)
        #[arg(long)]
        steer: Option<String>,
        /// The finished attempt a --steer attempt is seeded from
        #[arg(long)]
        from: Option<String>,
        /// Who asked for the attempt, when not the pipeline: `chat` (a chat
        /// agent). Recorded, mixed into the attempt id; its hand-offs live
        /// in the unit's `traces/chat/`; never scored as pipeline output
        #[arg(long, value_parser = ["chat"])]
        requester: Option<String>,
        /// A file (or `-`: stdin) holding the answer to the pending hand-off
        /// named by --answer-key (`external`, --requester=chat only): filed
        /// as its response when the resumed attempt asks for exactly that
        /// request
        #[arg(long, requires_all = ["answer_key", "requester"])]
        answer: Option<PathBuf>,
        /// The trace key (8 hex) of the hand-off --answer answers
        #[arg(long, requires = "answer")]
        answer_key: Option<String>,
        /// The answer's length in bytes (required with --answer=-): a read
        /// cut short — its writer gone midway — is refused, never filed
        #[arg(long, requires = "answer")]
        answer_bytes: Option<u64>,
    },
    /// Record a hand edit (exactly src/logic.rs and src/ffi.rs of DIR) as a
    /// labelled human attempt, judged by the oracle like a model reply; never
    /// promotes (`harness promote` does, explicitly)
    Override {
        /// Unit id from plan.toml
        unit: String,
        /// The directory holding src/logic.rs and src/ffi.rs
        dir: PathBuf,
        /// A short note recorded with the attempt
        #[arg(long)]
        note: Option<String>,
        #[command(flatten)]
        target: TargetArg,
        /// Run target/model-derived code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
    },
    /// Promote a recorded green migrate attempt into the unit's crate and
    /// verify it in place (the explicit act a review's Accept is)
    Promote {
        /// Unit id from plan.toml
        unit: String,
        /// The attempt id (`a-…`, or `a-….r2` for a later sample)
        attempt: String,
        #[command(flatten)]
        target: TargetArg,
        /// Replace the crate of an already verified unit, or re-promote an
        /// attempt that is already promoted
        #[arg(long)]
        replace: bool,
        /// Run target/model-derived code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
    },
    /// Benchmark suites: vendor, verify, score, regression-check
    Bench {
        #[command(subcommand)]
        cmd: bench::BenchCmd,
    },
    /// Generate a unit's differential driver through the configured LLM
    /// provider and self-validate it against the original C
    GenDriver {
        /// Unit id from plan.toml
        unit: String,
        #[command(flatten)]
        target: TargetArg,
        /// Provider profile override (built-in or user-level profile name)
        #[arg(long)]
        provider: Option<String>,
        /// Model override
        #[arg(long)]
        model: Option<String>,
        /// Replace the unit's existing GENERATED driver with a new green one
        #[arg(long)]
        promote: bool,
        /// Run target/model-derived code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
        /// Record a NEW sample when this live attempt already finished
        #[arg(long)]
        retry: bool,
        /// Pin the recorded attempt a `--provider replay` run verifies
        #[arg(long)]
        attempt: Option<String>,
    },
    /// Refresh the generated runtime view (AGENTS.md managed block)
    SyncRuntime {
        #[command(flatten)]
        target: TargetArg,
        /// Exit 1 if regeneration would change the block (CI mode)
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum FeaturesCmd {
    /// Write a starter migration/features/features.toml (never over one)
    Init {
        #[command(flatten)]
        target: TargetArg,
    },
    /// Save a new features.toml read from stdin, when it validates and the
    /// file is still the one --expect names
    Save {
        /// The blake3 of the file's current bytes, or `none` when there is none
        #[arg(long)]
        expect: String,
        /// The new file's length in bytes (a read cut short is refused)
        #[arg(long)]
        bytes: u64,
        #[command(flatten)]
        target: TargetArg,
    },
    /// Run every scenario on a probed copy of the C and record which
    /// functions each ran (migration/features/map.json)
    Map {
        #[command(flatten)]
        target: TargetArg,
        /// Run target code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
    },
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// Map the C files of one folder: each file's include folders, its
    /// compile and the symbols it defines and needs (writes nothing yet)
    Map {
        /// The project folder (its harness.toml's source_dir is mapped when
        /// it has one, else the folder itself)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Compile the project's code even though no sandbox is available
        #[arg(long)]
        allow_unsandboxed: bool,
        /// Map under this configuration of migration/map/config.toml
        /// (needed when it holds several)
        #[arg(long, value_name = "NAME")]
        configuration: Option<String>,
    },
}

#[derive(Subcommand)]
enum PerfCmd {
    /// Measure: the C alone, the program as it stands and every measurable
    /// unit on each workload (writes migration/perf/)
    Run {
        #[command(flatten)]
        target: TargetArg,
        /// Only this unit (repeatable)
        #[arg(long = "unit")]
        units: Vec<String>,
        /// Only this workload (repeatable)
        #[arg(long = "workload")]
        workloads: Vec<String>,
        /// Runs a side, 5 to 31 (over each workload's own)
        #[arg(long, value_parser = clap::value_parser!(u32).range(5..=31))]
        runs: Option<u32>,
        /// Only the program as it stands
        #[arg(long)]
        as_it_stands_only: bool,
        /// Accepted for symmetry; perf always runs the program in its own
        /// sandbox (macOS)
        #[arg(long)]
        allow_unsandboxed: bool,
    },
    /// Write a starter migration/perf/workloads.toml (never over one)
    Init {
        #[command(flatten)]
        target: TargetArg,
    },
    /// Save a new workloads.toml read from stdin, when it validates and the
    /// file is still the one --expect names
    Save {
        /// The blake3 of the file's current bytes, or `none` when there is none
        #[arg(long)]
        expect: String,
        /// The new file's length in bytes (a read cut short is refused)
        #[arg(long)]
        bytes: u64,
        #[command(flatten)]
        target: TargetArg,
    },
    /// Show every stored row's words and whether it is current
    Show {
        #[command(flatten)]
        target: TargetArg,
        /// Skip checking the computer and the compilers
        #[arg(long)]
        no_check: bool,
        /// Where no sandbox is available, still check the compilers
        /// (`cc --version` and `rustc -V` run as tool runs; on macOS they
        /// always run in the tool sandbox)
        #[arg(long)]
        allow_unsandboxed: bool,
    },
}

#[derive(Subcommand)]
enum StateCmd {
    /// Staleness report: facts vs tree, plan vs tree, verdicts vs tree
    Status {
        #[command(flatten)]
        target: TargetArg,
    },
}

/// Print a line to stdout (or, under `--json`, a `message` event), ignoring
/// EPIPE (`harness ... | head` must exit with a documented code, not a
/// broken-pipe panic).
pub(crate) fn out(line: String) {
    report::line(line);
}

/// Take the ledger's writer lock for a writing command
/// (docs/CLI-HARDENING.md §1); another holder is a `locked` error.
pub(crate) fn lock_ledger(ledger: &Ledger, command: &str) -> Result<WriterLock> {
    Ok(WriterLock::acquire(ledger, command)?)
}

/// On SIGINT/SIGTERM/SIGHUP: kill every live sandboxed process group, emit
/// the events-mode `result`, then die BY the signal (a normal exit 130
/// would make an interactive shell continue a loop over units). A failure
/// to install the handler leaves today's behaviour (the harness dies, the
/// children run to their own timeout) and is reported on stderr.
fn install_signal_handler() {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    use std::io::IsTerminal;
    // SIGHUP only for the case it is registered for — a terminal (or tmux
    // pane) still attached to our output. `nohup` sets SIGHUP to SIG_IGN and
    // redirects stdout/stderr away from the terminal, and installing a
    // handler would silently override that inherited disposition (std cannot
    // read it without `unsafe`); a detached run keeps its own.
    let mut sigs = vec![SIGINT, SIGTERM];
    if std::io::stdout().is_terminal() || std::io::stderr().is_terminal() {
        sigs.push(SIGHUP);
    }
    let mut signals = match signal_hook::iterator::Signals::new(&sigs) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(
                std::io::stderr(),
                "harness: cannot install the signal handler: {e}"
            );
            return;
        }
    };
    std::thread::spawn(move || {
        if let Some(sig) = signals.forever().next() {
            let name = match sig {
                SIGINT => "SIGINT",
                SIGTERM => "SIGTERM",
                _ => "SIGHUP",
            };
            let killed = harness_oracle::kill_live_process_groups();
            // A features map's random folder: its drop never runs when the
            // process dies by the signal.
            harness_oracle::remove_live_scratch_dirs();
            // Courtesy output only (docs/CLI-HARDENING.md §3 "best-effort"):
            // the main thread may be parked in a write on a full stdout pipe
            // holding the stdout mutex, and a closed stderr would make
            // `eprintln!` panic — neither may delay or prevent dying BY the
            // signal. Write from a helper, wait a bounded time, re-raise.
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            let helper = std::thread::Builder::new().spawn(move || {
                let _ = writeln!(
                    std::io::stderr(),
                    "harness: {name} — killed {killed} live process group(s); terminating"
                );
                report::result(128 + sig, Some(name));
                let _ = tx.send(());
            });
            if helper.is_ok() {
                let _ = rx.recv_timeout(std::time::Duration::from_millis(250));
            }
            let _ = signal_hook::low_level::emulate_default_handler(sig);
            std::process::exit(128 + sig);
        }
    });
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    install_signal_handler();
    // `--adopt` is the person's word for this run only: never carried into
    // a resume command.
    report::set_args(
        std::env::args()
            .skip(1)
            .filter(|a| a != "--json" && a != "--adopt")
            .collect(),
    );
    if cli.json {
        report::init(report::Mode::Json);
        let _ = harness_llm::progress::install(Box::new(report::Progress));
        let argv: Vec<String> = report::args().to_vec();
        let command = argv.first().cloned().unwrap_or_default();
        report::header(&command, argv.get(1..).unwrap_or(&[]));
    }
    let opened = cli.cmd.ledger_root();
    let fresh = match &opened {
        Some((root, scope)) => open_ledger(root, *scope, cli.adopt),
        None => Ok(false),
    };
    let (result, fresh) = match fresh {
        Err(e) => (Err(e), false),
        Ok(fresh) => (run(cli.cmd), fresh),
    };
    // The first command that made this folder's ledger records it as made
    // on this computer.
    if let (true, Some((root, scope))) = (fresh, &opened) {
        if let Err(e) = adopt::record_created(root, *scope) {
            let _ = writeln!(
                std::io::stderr(),
                "harness: could not record {} as made on this computer ({}); the next command \
                 will ask for `--adopt`",
                root.display(),
                report::terminal_safe(&e.to_string())
            );
        }
    }
    let code = match result {
        Ok(code) => code,
        Err(e) => {
            // A closed stderr must not turn exit 1 into a panic (101).
            let _ = writeln!(
                std::io::stderr(),
                "error: {}",
                report::terminal_safe(&format!("{e:#}"))
            );
            report::error(&e);
            1
        }
    };
    report::result(i32::from(code), None);
    ExitCode::from(code)
}

/// Run one command.
fn run(cmd: Cmd) -> Result<u8> {
    match cmd {
        Cmd::Scan { target } => cmd_scan(target),
        Cmd::Plan { target } => cmd_plan(target),
        Cmd::Verify {
            unit,
            target,
            allow_unsandboxed,
        } => cmd_verify(unit, target, allow_unsandboxed),
        Cmd::State {
            cmd: StateCmd::Status { target },
        } => cmd_status(target),
        Cmd::Features { cmd } => match cmd {
            FeaturesCmd::Init { target } => features::cmd_init(target),
            FeaturesCmd::Save {
                expect,
                bytes,
                target,
            } => features::cmd_save(target, expect, bytes),
            FeaturesCmd::Map {
                target,
                allow_unsandboxed,
            } => features::cmd_map(target, allow_unsandboxed),
        },
        Cmd::Project {
            cmd:
                ProjectCmd::Map {
                    target,
                    allow_unsandboxed,
                    configuration,
                },
        } => project::cmd_map(target, allow_unsandboxed, configuration),
        Cmd::Perf { cmd } => match cmd {
            PerfCmd::Run {
                target,
                units,
                workloads,
                runs,
                as_it_stands_only,
                allow_unsandboxed: _,
            } => perf::cmd_run(target, units, workloads, runs, as_it_stands_only),
            PerfCmd::Init { target } => perf::cmd_init(target),
            PerfCmd::Save {
                expect,
                bytes,
                target,
            } => perf::cmd_save(target, expect, bytes),
            PerfCmd::Show {
                target,
                no_check,
                allow_unsandboxed,
            } => perf::cmd_show(target, no_check, allow_unsandboxed),
        },
        Cmd::Detect { target } => cmd_detect(target),
        Cmd::Observe { target } => cmd_observe(target),
        Cmd::Review {
            finding,
            uphold_dismiss,
            reinstate,
            note,
            target,
        } => cmd_review(finding, uphold_dismiss, reinstate, note, target),
        Cmd::Migrate {
            unit,
            target,
            provider,
            model,
            promote,
            no_promote,
            allow_unsandboxed,
            retry,
            attempt,
            steer,
            from,
            requester,
            answer,
            answer_key,
            answer_bytes,
        } => cmd_migrate(MigrateArgs {
            unit,
            target,
            provider,
            model,
            promote,
            no_promote,
            allow_unsandboxed,
            retry,
            attempt,
            steer,
            from,
            requester,
            answer,
            answer_key,
            answer_bytes,
        }),
        Cmd::Override {
            unit,
            dir,
            note,
            target,
            allow_unsandboxed,
        } => hand_edit::cmd_override(unit, dir, note, target, allow_unsandboxed),
        Cmd::Promote {
            unit,
            attempt,
            target,
            replace,
            allow_unsandboxed,
        } => promote::cmd_promote(unit, attempt, target, replace, allow_unsandboxed),
        Cmd::SyncRuntime { target, check } => cmd_sync_runtime(target, check),
        Cmd::Bench { cmd } => bench::run(cmd),
        Cmd::GenDriver {
            unit,
            target,
            provider,
            model,
            promote,
            allow_unsandboxed,
            retry,
            attempt,
        } => gen_driver::cmd_gen_driver(gen_driver::GenDriverArgs {
            unit,
            target,
            provider,
            model,
            promote,
            allow_unsandboxed,
            retry,
            attempt,
        }),
    }
}

fn cmd_scan(target: TargetArg) -> Result<u8> {
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = lock_ledger(&ledger, "scan")?;
    let facts = scan_target(&ctx)?;
    out(format!(
        "scan: {} files, {} symbols, {} refs -> {}",
        facts.files.len(),
        facts.symbols.len(),
        facts.refs.len(),
        ledger.facts_path().display()
    ));
    Ok(0)
}

/// Scan `ctx` and write `facts.jsonl` (the body of `harness scan`).
pub(crate) fn scan_target(ctx: &TargetContext) -> Result<Facts> {
    let ledger = Ledger::of(ctx);
    let (facts, notes) = harness_scan::CFrontend.scan_reporting(ctx)?;
    for (path, why) in &notes.skipped {
        // Never read (a FIFO or a device would block the scan forever), or
        // could not be: a note of that path, never a stop.
        out(format!(
            "scan: skipped {}: {}",
            harness_core::text::safe_line(
                &path
                    .strip_prefix(&ctx.root)
                    .unwrap_or(path)
                    .to_string_lossy()
            ),
            harness_core::text::safe_line(why)
        ));
    }
    for path in &notes.too_large {
        out(format!(
            "scan: {} is over 8 MiB: recorded, not parsed",
            harness_core::text::safe_line(path)
        ));
    }
    for path in &notes.not_utf8 {
        out(format!(
            "scan: {} is not UTF-8: parsed from its bytes",
            harness_core::text::safe_line(path)
        ));
    }
    std::fs::create_dir_all(ledger.dir()).context("creating migration dir")?;
    facts.store(&ledger.facts_path())?;
    Ok(facts)
}

/// The typed refusal for stale facts (`error.kind = "stale"`).
fn facts_stale(stale: usize) -> Error {
    Error::Stale {
        subject: "facts.jsonl".into(),
        hint: format!("{stale} file(s) changed on disk; run `harness scan` first"),
    }
}

/// Count facts file records whose hash no longer matches the working tree,
/// and — for a file-list target — the files a scan would record that the
/// facts lack (a listed file, or a header an include would now find).
pub(crate) fn stale_fact_files(ctx: &TargetContext, facts: &Facts) -> usize {
    facts
        .files
        .iter()
        .filter(|f| {
            hash::file_hash(&ctx.root.join(&f.path))
                .map(|h| h != f.hash)
                .unwrap_or(true)
        })
        .count()
        + harness_core::features::unrecorded_program_files(ctx, facts).len()
}

fn cmd_plan(target: TargetArg) -> Result<u8> {
    let ctx = target.load()?;
    let _lock = lock_ledger(&Ledger::of(&ctx), "plan")?;
    let (changes, order_line, units) = plan_target(&ctx)?;
    if changes.is_empty() {
        out(format!("plan: no changes ({units} units)"));
    } else {
        for c in &changes {
            out(format!("plan: {c}"));
        }
    }
    out(format!("plan: execution order: {order_line}"));
    Ok(0)
}

/// Reconcile and write `plan.toml` (the body of `harness plan`): returns the
/// change lines, the execution-order line and the unit count.
pub(crate) fn plan_target(ctx: &TargetContext) -> Result<(Vec<String>, String, usize)> {
    let ledger = Ledger::of(ctx);
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    // Planning from stale facts would write stale hashes and strand verify
    // in a refusal loop — refuse up front instead.
    let stale = stale_fact_files(ctx, &facts);
    if stale > 0 {
        return Err(facts_stale(stale).into());
    }
    let mut computed = planner::compute_units(&facts)?;
    let plan_path = ledger.plan_path();
    let existing_text = if plan_path.exists() {
        let existing = Plan::load(&plan_path)?;
        plan::adopt_existing_ids(&mut computed, &existing);
        Some(std::fs::read_to_string(&plan_path).context("re-reading plan")?)
    } else {
        None
    };
    // Reconcile in memory, validate, and only then write (a corrupt plan
    // must never reach disk).
    let (content, changes) = plan::reconcile_to_string(
        &plan_path,
        existing_text.as_deref(),
        &ctx.config.target.name,
        &computed,
    )?;
    let reconciled = Plan::parse(&plan_path, &content)?;
    let order = reconciled
        .execution_order()
        .context("reconciled plan failed validation; plan.toml was NOT modified")?;
    let order_line = order
        .iter()
        .map(|u| u.id.as_str())
        .collect::<Vec<_>>()
        .join(" -> ");
    harness_core::ledger::write_atomic(&plan_path, content.as_bytes())?;
    Ok((changes, order_line, computed.len()))
}

/// Every command that builds or runs target- or model-derived code refuses
/// on platforms without a sandbox unless the user explicitly accepts the risk
/// (docs/SCHEMAS.md "Trust boundaries") — regardless of provider: an
/// `external` candidate and the target's own driver.c run just the same.
pub(crate) fn require_sandbox(allow_unsandboxed: bool, what: &str) -> Result<()> {
    if harness_oracle::sandbox_mode() == "none" && !allow_unsandboxed {
        bail!(
            "no sandbox is available on this platform; `{what}` builds and runs target- and \
             model-derived code unconfined — pass --allow-unsandboxed to accept that"
        );
    }
    Ok(())
}

/// [`safe_ledger_dir`] for a run that must create nothing: every component
/// must already be a real directory (never a symlink).
fn existing_ledger_dir(ctx: &TargetContext, components: &[&str]) -> Result<PathBuf> {
    let mut cur = ctx.root.clone();
    for comp in ledger_components(ctx, components) {
        cur = cur.join(comp);
        match std::fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_dir() => {}
            _ => bail!("{} is not a real directory", cur.display()),
        }
    }
    Ok(cur)
}

/// The parts from the target root down to `components` inside its ledger:
/// the ledger's own (`migration`, or `migration/tools/<id>`), then them.
fn ledger_components(ctx: &TargetContext, components: &[&str]) -> Vec<String> {
    let mut parts = ctx.ledger_parts();
    parts.extend(components.iter().map(|c| c.to_string()));
    parts
}

/// A directory inside the target's ledger (`components` below the ledger
/// folder) that is guaranteed not to be (or pass through) a symlink: created
/// level by level under the canonical target root, refusing any component
/// that is not a real directory. Target-owned trees are hostile — a
/// committed `traces -> /elsewhere` must not redirect harness writes.
pub(crate) fn safe_ledger_dir(ctx: &TargetContext, components: &[&str]) -> Result<PathBuf> {
    let mut cur = ctx.root.clone();
    for comp in ledger_components(ctx, components) {
        cur = cur.join(comp);
        match std::fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                bail!(
                    "{} is not a real directory; refusing to write through it",
                    cur.display()
                )
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&cur).with_context(|| format!("creating {}", cur.display()))?;
            }
            Err(e) => return Err(e).with_context(|| format!("inspecting {}", cur.display())),
        }
    }
    Ok(cur)
}

fn cmd_verify(unit_id: String, target: TargetArg, allow_unsandboxed: bool) -> Result<u8> {
    require_sandbox(allow_unsandboxed, "harness verify")?;
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = lock_ledger(&ledger, &format!("verify {unit_id}"))?;
    let plan_path = ledger.plan_path();
    let plan_doc = Plan::load(&plan_path)?;
    // Structural validation on every load (docs/SCHEMAS.md): never run the
    // oracle against a plan with duplicate ids, missing deps, or cycles.
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid; fix it before verifying")?;
    let unit = plan_doc.unit(&unit_id)?;
    promote::recover_promotion(&ctx, &ledger, unit)?;
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;

    // Stale-plan refusal (docs/SCHEMAS.md): the tree must match what was planned.
    let closure = facts.include_closure(&unit.files);
    let current = hash::file_set_hash_on_disk(&ctx.root, &closure)?;
    if current != unit.source_hash {
        return Err(Error::Stale {
            subject: format!("unit `{unit_id}`"),
            hint: format!(
                "source changed since planning (plan {} vs tree {current}); run `harness scan`, \
                 then `harness plan`, review the diff, then re-verify",
                unit.source_hash
            ),
        }
        .into());
    }

    // The person's features, loaded once for this run (docs/FEATURES-DESIGN.md
    // §2.2): a file with an error is said and never blocks the verify.
    let features = harness_core::features::FeatureSnapshot::load(&ctx);
    announce_features(&features);
    let verdict = match unit.oracle_kind() {
        Some("c-abi-differential") => {
            let strategy = harness_oracle::CAbiDifferential;
            strategy.verify_with(&ctx, unit, &features)?
        }
        Some(kind) => bail!("unknown oracle kind `{kind}` for unit `{unit_id}`"),
        None => bail!("unit `{unit_id}` has no [unit.oracle] configured"),
    };
    announce_skips(&verdict);

    verdict.store(&ledger.verdict_latest_path(&unit_id))?;
    harness_core::ledger::write_atomic(
        &ledger.verdict_md_path(&unit_id),
        verdict.render_md().as_bytes(),
    )?;
    report::verdict(&unit_id, &verdict, &ledger.verdict_latest_path(&unit_id));
    for c in &verdict.checks {
        out(format!(
            "verify: [{}] {} — {}",
            if c.passed { "PASS" } else { "FAIL" },
            c.name,
            c.detail
        ));
    }
    if verdict.green {
        verdict.store(&ledger.verdict_last_green_path(&unit_id))?;
        plan::set_status(&plan_path, &unit_id, UnitStatus::Verified)?;
        out(format!("verify: {unit_id} GREEN — status set to verified"));
        Ok(0)
    } else {
        match unit.status {
            UnitStatus::Verified => {
                plan::set_status(&plan_path, &unit_id, UnitStatus::InProgress)?;
                out(format!(
                    "verify: {unit_id} RED — status demoted verified -> in-progress"
                ));
            }
            UnitStatus::Merged => {
                // Never auto-demote a merged unit; surface loudly instead.
                out(format!(
                    "verify: {unit_id} RED — unit is `merged` but its latest \
                     evidence is now red; investigate (status left untouched)"
                ));
            }
            _ => out(format!("verify: {unit_id} RED")),
        }
        Ok(EXIT_ORACLE_RED)
    }
}

/// Say, before the oracle runs, what the person's features will do
/// (docs/FEATURES-DESIGN.md §5.5).
pub(crate) fn announce_features(features: &harness_core::features::FeatureSnapshot) {
    use harness_core::features::FeatureSnapshot;
    match features {
        FeatureSnapshot::None => {}
        FeatureSnapshot::Invalid(why) => out(format!(
            "verify: your features file has an error — no feature scenario runs ({})",
            harness_core::features::one_line(why)
        )),
        FeatureSnapshot::Valid { features, .. } if !features.scenarios.is_empty() => {
            let n = features.scenarios.len();
            out(format!(
                "verify: running your {n} feature scenario{} after the other checks",
                if n == 1 { "" } else { "s" }
            ));
        }
        FeatureSnapshot::Valid { .. } => {}
    }
}

/// Say which feature scenarios a verdict could not run, and why, in words
/// (docs/FEATURES-DESIGN.md §6.1).
pub(crate) fn announce_skips(verdict: &harness_core::Verdict) {
    for entry in &verdict.inputs.features_skipped {
        if let Some((feature, scenario, reason)) = harness_core::features::parse_skip(entry) {
            out(format!(
                "verify: skipped {feature}/{scenario}: {} — {}",
                reason.words(),
                reason.what_to_do()
            ));
        }
    }
}

fn cmd_status(target: TargetArg) -> Result<u8> {
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);

    let facts = match Facts::load(&ledger.facts_path()) {
        Ok(f) => f,
        Err(e) if e.is_not_found() => {
            out("status: no facts — run `harness scan`".into());
            return Ok(0);
        }
        // Parse errors and newer-schema refusals must surface, not read as
        // "no facts" (docs/SCHEMAS.md versioning rules).
        Err(e) => return Err(e.into()),
    };
    let stale_files = stale_fact_files(&ctx, &facts);
    out(format!(
        "status: facts {} ({} files, {} stale vs tree)",
        if stale_files == 0 {
            "fresh"
        } else {
            "STALE — run `harness scan`"
        },
        facts.files.len(),
        stale_files
    ));
    #[derive(serde::Serialize)]
    struct FactsEvent {
        k: &'static str,
        files: usize,
        stale: usize,
    }
    report::event(&FactsEvent {
        k: "facts",
        files: facts.files.len(),
        stale: stale_files,
    });

    let plan_path = ledger.plan_path();
    if !plan_path.exists() {
        out("status: no plan — run `harness plan`".into());
        return Ok(0);
    }
    let plan_doc = Plan::load(&plan_path)?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid")?;
    #[derive(serde::Serialize)]
    struct UnitEvent<'a> {
        k: &'static str,
        #[serde(flatten)]
        report: &'a harness_core::status::UnitReport,
    }
    // Today's feature digests, once for every unit (docs/FEATURES-DESIGN.md
    // §2.4); nothing is hashed without a features file.
    let features_now = harness_core::features::FeaturesNow::compute(
        &ctx,
        &facts,
        &harness_core::features::FeatureSnapshot::load(&ctx),
    );
    for unit in &plan_doc.units {
        // One computation renders both surfaces (docs/CLI-HARDENING.md §4).
        let r =
            harness_core::status::unit_report(&ctx, &ledger, &facts, unit, features_now.as_ref())?;
        out(r.render_line());
        if let Some(line) = r.render_attempts_line() {
            out(line);
        }
        report::event(&UnitEvent {
            k: "unit",
            report: &r,
        });
    }
    Ok(0)
}

// ---------- M2: observer commands ----------

fn facts_records_hash(facts: &Facts) -> String {
    let pairs: Vec<(String, String)> = facts
        .files
        .iter()
        .map(|f| (f.path.clone(), f.hash.clone()))
        .collect();
    hash::file_set_hash(&pairs)
}

fn cmd_detect(target: TargetArg) -> Result<u8> {
    use harness_core::observer::{FindingsFile, ObserverPaths};
    use harness_core::traits::Detector;
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = lock_ledger(&ledger, "detect")?;
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    let stale = stale_fact_files(&ctx, &facts);
    if stale > 0 {
        return Err(facts_stale(stale).into());
    }
    let suite = harness_detect::CTreeSitterSuite;
    let findings = suite.detect(&ctx, &facts)?;
    let file = FindingsFile {
        detector_suite: suite.name().to_string(),
        facts_hash: facts_records_hash(&facts),
        findings,
    };
    let path = ObserverPaths::findings(&ledger);
    file.store(&path)?;
    let mut by_category: std::collections::BTreeMap<&str, usize> = Default::default();
    for f in &file.findings {
        *by_category.entry(f.category.as_str()).or_insert(0) += 1;
    }
    out(format!(
        "detect: {} finding(s) -> {}",
        file.findings.len(),
        path.display()
    ));
    for (cat, n) in by_category {
        out(format!("detect:   {cat}: {n}"));
    }
    Ok(0)
}

/// Everything `observe` needs, loaded and freshness-checked.
type ObserverInputs = (
    Facts,
    Plan,
    harness_core::observer::FindingsFile,
    Vec<harness_core::observer::Finding>,
    harness_core::observer::TriageFile,
    Vec<harness_core::observer::Review>,
);

fn observer_inputs(ctx: &TargetContext, ledger: &Ledger) -> Result<ObserverInputs> {
    use harness_core::observer::{self, ObserverPaths};
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    let plan_doc = Plan::load(&ledger.plan_path())?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid")?;
    let findings = harness_core::observer::FindingsFile::load(&ObserverPaths::findings(ledger))
        .context("loading findings (run `harness detect` first)")?;
    if findings.facts_hash != facts_records_hash(&facts) {
        return Err(Error::Stale {
            subject: "findings.jsonl".into(),
            hint: "it is bound to different facts; run `harness detect`".into(),
        }
        .into());
    }
    for f in &findings.findings {
        let now = hash::file_hash(&ctx.root.join(&f.file))
            .with_context(|| format!("hashing {}", f.file))?;
        if now != f.file_hash {
            return Err(Error::Stale {
                subject: format!("finding {}", f.id),
                hint: format!("{} changed since detect; run `harness detect`", f.file),
            }
            .into());
        }
    }
    let annotations = observer::load_annotations(&ObserverPaths::annotations(ledger))?;
    let triage = harness_core::observer::TriageFile::load(&ObserverPaths::triage(ledger))?;
    let reviews = observer::load_reviews(&ObserverPaths::reviews(ledger))?;
    Ok((facts, plan_doc, findings, annotations, triage, reviews))
}

/// `observe`'s `awaiting` hint: the target attached (a relative target
/// named `-x` must not become a flag), and its tool.
fn observe_resume(target: &TargetArg) -> String {
    format!("harness observe{}", target.resume_args())
}

fn cmd_observe(target: TargetArg) -> Result<u8> {
    use harness_core::observer::{self, ObserverPaths};
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = lock_ledger(&ledger, "observe")?;
    let (facts, plan_doc, findings, annotations, _, reviews) = observer_inputs(&ctx, &ledger)?;

    let traces = safe_ledger_dir(&ctx, &["observer", "traces"])?;
    let llm = &ctx.config.llm;
    let resolved = harness_llm::providers::resolve(&llm.provider, &traces)?;
    let outcome = match harness_llm::run_triage(
        &resolved,
        &llm.model,
        llm.max_tokens,
        &ctx,
        &facts,
        &plan_doc,
        &findings,
        &traces,
    ) {
        Ok(o) => o,
        Err(e @ Error::Awaiting { .. }) => {
            eprintln!("{e:#}");
            eprintln!(
                "observe: external provider mode — supply the response file(s) under {} and re-run",
                traces.display()
            );
            if let Error::Awaiting { path, .. } = &e {
                report::event(&report::Awaiting {
                    k: "awaiting",
                    attempt: None,
                    path: path.display().to_string(),
                    resume: observe_resume(&target),
                    args: report::args_without_answer(),
                    request_key: report::request_key_of(path),
                });
            }
            return Err(e.into());
        }
        Err(e) => return Err(e.into()),
    };
    outcome.triage.store(&ObserverPaths::triage(&ledger))?;

    let mut all_findings: Vec<harness_core::observer::Finding> = findings.findings.clone();
    all_findings.extend(annotations.iter().cloned());
    let risk = harness_core::risk::score_units(
        &facts,
        &plan_doc,
        &all_findings,
        &outcome.triage,
        &reviews,
    );
    let rendered = observer::render_observations(&observer::ObservationsInput {
        findings: &findings,
        annotations: &annotations,
        triage: &outcome.triage,
        reviews: &reviews,
        plan: &plan_doc,
        facts: &facts,
        risk: &risk,
    })?;
    harness_core::ledger::write_atomic(&ObserverPaths::observations(&ledger), rendered.as_bytes())?;
    let usage_in: u64 = outcome.usage.iter().map(|(_, i, _)| i).sum();
    let usage_out: u64 = outcome.usage.iter().map(|(_, _, o)| o).sum();
    out(format!(
        "observe: {} verdict(s) via `{}` -> {} (tokens in/out: {}/{})",
        outcome.triage.verdicts.len(),
        resolved.adapter.name(),
        ObserverPaths::observations(&ledger).display(),
        usage_in,
        usage_out
    ));
    for r in risk.iter().take(5) {
        out(format!("observe: risk {} {}", r.score, r.unit));
    }
    Ok(0)
}

fn cmd_review(
    finding: String,
    uphold_dismiss: bool,
    reinstate: bool,
    note: String,
    target: TargetArg,
) -> Result<u8> {
    use harness_core::observer::{self, ObserverPaths};
    if uphold_dismiss == reinstate {
        bail!("pass exactly one of --uphold-dismiss or --reinstate");
    }
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = lock_ledger(&ledger, &format!("review {finding}"))?;
    let findings = harness_core::observer::FindingsFile::load(&ObserverPaths::findings(&ledger))
        .context("loading findings (run `harness detect` first)")?;
    let annotations = observer::load_annotations(&ObserverPaths::annotations(&ledger))?;
    if !findings.findings.iter().any(|f| f.id == finding)
        && !annotations.iter().any(|f| f.id == finding)
    {
        bail!("unknown finding `{finding}`");
    }
    let action = if uphold_dismiss {
        "uphold-dismiss"
    } else {
        "reinstate"
    };
    observer::append_review(
        &ObserverPaths::reviews(&ledger),
        &observer::Review {
            finding: finding.clone(),
            action: action.into(),
            note,
        },
    )?;
    out(format!(
        "review: {finding} {action} recorded — re-run `harness observe` to re-render"
    ));
    Ok(0)
}

fn cmd_sync_runtime(target: TargetArg, check: bool) -> Result<u8> {
    use harness_core::observer::{self, ObserverPaths};
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = if check {
        None
    } else {
        Some(lock_ledger(&ledger, "sync-runtime")?)
    };
    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    let plan_doc = Plan::load(&ledger.plan_path())?;
    // Risk from whatever observer state exists: a MISSING file is fine
    // (empty), but parse errors and newer-schema refusals must propagate —
    // the agent-facing view must never be built from silently-dropped data.
    let findings =
        match harness_core::observer::FindingsFile::load(&ObserverPaths::findings(&ledger)) {
            Ok(f) => f,
            Err(e) if e.is_not_found() => Default::default(),
            Err(e) => return Err(e.into()),
        };
    let annotations = observer::load_annotations(&ObserverPaths::annotations(&ledger))?;
    let triage = harness_core::observer::TriageFile::load(&ObserverPaths::triage(&ledger))?;
    let reviews = observer::load_reviews(&ObserverPaths::reviews(&ledger))?;
    let mut all_findings = findings.findings.clone();
    all_findings.extend(annotations);
    let risk = harness_core::risk::score_units(&facts, &plan_doc, &all_findings, &triage, &reviews);

    // One block per target: a folder-form target's, and each mapped tool's
    // marked with its id; the other tools' blocks are kept as they are.
    // (Two tools synced at the same moment could race on the shared
    // AGENTS.md: the project lock of step (b) will cover it.)
    let ledger_rel = ctx.ledger_rel();
    let at = harness_core::runtime_view::BlockTarget {
        tool: ctx.tool.as_deref(),
        ledger: &ledger_rel,
    };
    let body = harness_core::runtime_view::render_block_body(
        &ctx.config.target.name,
        at,
        &plan_doc,
        &risk,
    );
    let block = harness_core::runtime_view::wrap_block(&body, at);
    let agents_path = ctx.root.join("AGENTS.md");
    let existing = std::fs::read_to_string(&agents_path).ok();
    let updated = harness_core::runtime_view::apply(existing.as_deref(), at, &block)?;
    if check {
        if existing.as_deref() == Some(updated.as_str()) {
            out("sync-runtime: up to date".into());
            return Ok(0);
        }
        let run = match &ctx.tool {
            None => "harness sync-runtime".to_string(),
            Some(id) => format!("harness sync-runtime --tool {id}"),
        };
        eprintln!("sync-runtime: AGENTS.md managed block is out of date; run `{run}`");
        return Ok(1);
    }
    harness_core::ledger::write_atomic(&agents_path, updated.as_bytes())?;
    // Claude Code bridge: CLAUDE.md imports AGENTS.md (per §14.3 spike evidence).
    let claude_path = ctx.root.join("CLAUDE.md");
    let claude = std::fs::read_to_string(&claude_path).unwrap_or_default();
    if !claude.contains("@AGENTS.md") {
        let mut updated_claude = claude;
        if !updated_claude.is_empty() && !updated_claude.ends_with('\n') {
            updated_claude.push('\n');
        }
        updated_claude.push_str("@AGENTS.md\n");
        harness_core::ledger::write_atomic(&claude_path, updated_claude.as_bytes())?;
    }
    out(format!("sync-runtime: {} updated", agents_path.display()));
    Ok(0)
}

// ---------- M3: executor command ----------

/// The unit's confirmed hazards as `harness migrate` puts them in the
/// translate prompt (annotations are implicitly confirmed). Shared with
/// `bench check --replay`, which must pose the byte-identical prompt.
pub(crate) fn confirmed_hazards(
    ledger: &Ledger,
    plan_doc: &Plan,
    facts: &Facts,
    unit_id: &str,
) -> Result<Vec<harness_core::observer::Finding>> {
    use harness_core::observer::{self, FindingState, ObserverPaths};
    let mut hazards = Vec::new();
    let findings = match observer::FindingsFile::load(&ObserverPaths::findings(ledger)) {
        Ok(f) => f.findings,
        Err(e) if e.is_not_found() => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    let annotations = observer::load_annotations(&ObserverPaths::annotations(ledger))?;
    let triage = observer::TriageFile::load(&ObserverPaths::triage(ledger))?;
    let reviews = observer::load_reviews(&ObserverPaths::reviews(ledger))?;
    for f in findings.iter().chain(annotations.iter()) {
        let affects = observer::affected_units(&f.file, plan_doc, facts).contains(&unit_id);
        let state = observer::finding_state(f, &triage, &reviews);
        if affects && matches!(state, FindingState::Confirmed | FindingState::Reinstated) {
            hazards.push(f.clone());
        }
    }
    Ok(hazards)
}

/// Arguments of `harness migrate`.
struct MigrateArgs {
    unit: String,
    target: TargetArg,
    provider: Option<String>,
    model: Option<String>,
    promote: bool,
    no_promote: bool,
    allow_unsandboxed: bool,
    retry: bool,
    attempt: Option<String>,
    steer: Option<String>,
    from: Option<String>,
    requester: Option<String>,
    answer: Option<PathBuf>,
    answer_key: Option<String>,
    answer_bytes: Option<u64>,
}

impl MigrateArgs {
    /// The command line that resumes this run, as a human would type it:
    /// every value shell-quoted and ATTACHED (`--steer='- keep it'`: clap
    /// reads a separate word that starts with `-` as a flag, so a note like
    /// `- use iter()` would not survive the round trip), the promotion and
    /// steer flags kept (the `awaiting` hint and event carry it), the
    /// requester too — never `--answer`/`--answer-key` (an answer is filed
    /// once). Global flags (`--json`) are not repeated; a client re-runs its
    /// own argv (the event's `args`).
    fn resume_command(&self) -> String {
        let q = report::shell_quote;
        let mut cmd = format!("harness migrate {}", q(&self.unit));
        cmd.push_str(&self.target.resume_args());
        if let Some(p) = &self.provider {
            cmd.push_str(&format!(" --provider={}", q(p)));
        }
        if let Some(m) = &self.model {
            cmd.push_str(&format!(" --model={}", q(m)));
        }
        if self.promote {
            cmd.push_str(" --promote");
        }
        if self.no_promote {
            cmd.push_str(" --no-promote");
        }
        if self.allow_unsandboxed {
            cmd.push_str(" --allow-unsandboxed");
        }
        if self.retry {
            cmd.push_str(" --retry");
        }
        if let Some(a) = &self.attempt {
            cmd.push_str(&format!(" --attempt={}", q(a)));
        }
        if let Some(from) = &self.from {
            cmd.push_str(&format!(" --from={}", q(from)));
        }
        if let Some(note) = &self.steer {
            cmd.push_str(&format!(" --steer={}", q(note)));
        }
        if let Some(requester) = &self.requester {
            cmd.push_str(&format!(" --requester={}", q(requester)));
        }
        cmd
    }
}

/// Largest `--answer` file (as harness-mcp's `harness_answer` took).
const MAX_ANSWER_BYTES: u64 = 512 * 1024;

/// `--answer FILE --answer-key KEY [--answer-bytes N]`, checked: KEY 8
/// lowercase hex; FILE (or `-`: stdin, as harness-mcp passes it — the answer
/// never lands in a file — never a terminal, and framed by N, so a writer
/// killed midway leaves a short read that is refused, §R5 F2) a regular file
/// of ≤ [`MAX_ANSWER_BYTES`], read through a checked handle; exactly N bytes
/// when N is given; UTF-8, not blank. Every refusal is typed
/// `answer-refused`.
fn read_answer(file: &Path, key: &str, expected: Option<u64>) -> Result<(String, String)> {
    use std::io::{IsTerminal, Read};
    let refuse = |why: String| {
        anyhow::Error::new(Error::AnswerRefused {
            why: format!("{}: {why}", file.display()),
        })
    };
    if !harness_core::traces::is_trace_key(key) {
        return Err(Error::AnswerRefused {
            why: format!(
                "--answer-key {:?} is not a trace key (8 lowercase hex digits)",
                key.chars().take(16).collect::<String>()
            ),
        }
        .into());
    }
    let bytes = if file == Path::new("-") {
        if std::io::stdin().is_terminal() {
            return Err(refuse(
                "stdin is a terminal: pipe the answer in, or pass a file".into(),
            ));
        }
        if expected.is_none() {
            return Err(refuse(
                "--answer=- needs --answer-bytes (the answer's length: a read cut short is \
                 refused)"
                    .into(),
            ));
        }
        let mut bytes = Vec::new();
        std::io::stdin()
            .lock()
            .take(MAX_ANSWER_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| refuse(e.to_string()))?;
        if bytes.len() as u64 > MAX_ANSWER_BYTES {
            return Err(refuse(format!("longer than {MAX_ANSWER_BYTES} bytes")));
        }
        bytes
    } else {
        // Looked at once (never through a symlink), opened without
        // blocking, the handle checked to be that file, read bounded (§R
        // CS-6, CR-11).
        harness_core::ledger::read_regular(file, MAX_ANSWER_BYTES).map_err(|e| {
            anyhow::Error::new(Error::AnswerRefused {
                why: format!("--answer {e}"),
            })
        })?
    };
    if let Some(expected) = expected.filter(|&n| n != bytes.len() as u64) {
        return Err(refuse(format!(
            "{} bytes, not the {expected} --answer-bytes names: cut short or changed, not filed",
            bytes.len()
        )));
    }
    let text = String::from_utf8(bytes).map_err(|_| refuse("it must be UTF-8".into()))?;
    if text.trim().is_empty() {
        return Err(refuse("the answer is empty".into()));
    }
    Ok((key.to_string(), text))
}

fn cmd_migrate(args: MigrateArgs) -> Result<u8> {
    let resume = args.resume_command();
    let MigrateArgs {
        unit: unit_id,
        target,
        provider: provider_flag,
        model: model_flag,
        promote: promote_flag,
        no_promote,
        allow_unsandboxed,
        retry,
        attempt,
        steer,
        from,
        requester,
        answer,
        answer_key,
        answer_bytes,
    } = args;
    require_sandbox(allow_unsandboxed, "harness migrate")?;
    // `--answer`'s own checks, before the lock and anything written
    // (docs/CHAT-PANE-DESIGN.md §4.3); the attempt's are the executor's.
    let answer = match (&answer, &answer_key) {
        (Some(file), Some(key)) => Some(read_answer(file, key, answer_bytes)?),
        _ => None,
    };
    let ctx = target.load()?;
    let ledger = Ledger::of(&ctx);
    let _lock = lock_ledger(&ledger, &format!("migrate {unit_id}"))?;
    let plan_path = ledger.plan_path();
    let plan_doc = Plan::load(&plan_path)?;
    plan_doc
        .execution_order()
        .context("plan.toml is structurally invalid; fix it before migrating")?;
    let unit = plan_doc.unit(&unit_id)?;
    promote::recover_promotion(&ctx, &ledger, unit)?;

    let facts =
        Facts::load(&ledger.facts_path()).context("loading facts (run `harness scan` first)")?;
    // The plan's staleness rule and R6 (a generated driver must carry a
    // FRESH green validation) — shared with `harness promote`.
    promote::migrate_preconditions(&ctx, &ledger, &facts, unit, "migrate")?;
    // A steer attempt names its seed explicitly: the ledger defines no order
    // over a unit's attempts, so there is no "latest" to default to.
    if steer.is_some() != from.is_some() {
        let (unit_source, driver) = harness_core::attempts::current_binding(&ctx, &facts, unit)?;
        let mut finished: Vec<String> =
            harness_core::attempts::load_unit_attempts(&ledger, &unit_id)?
                .into_iter()
                .filter(|r| {
                    r.outcome != "in-progress" && r.unit_source == unit_source && r.driver == driver
                })
                .map(|r| r.id)
                .collect();
        finished.sort();
        bail!(
            "--steer and --from go together: pass --from <ATTEMPT> with --steer <NOTE>; finished \
             attempts of `{unit_id}` bound to the current inputs: {}",
            if finished.is_empty() {
                "none".to_string()
            } else {
                finished.join(", ")
            }
        );
    }

    // Stage routing (§13.2): flag > [llm.migrate] > [llm].
    let llm = &ctx.config.llm;
    let stage = llm.migrate.as_ref();
    let provider_name = provider_flag
        .or_else(|| stage.and_then(|m| m.provider.clone()))
        .unwrap_or_else(|| llm.provider.clone());
    let model = model_flag
        .or_else(|| stage.and_then(|m| m.model.clone()))
        .unwrap_or_else(|| llm.model.clone());
    let max_tokens = stage.and_then(|m| m.max_tokens).unwrap_or(llm.max_tokens);
    let max_repairs = stage.and_then(|m| m.max_repairs).unwrap_or(3);
    let promote_on_green = stage.and_then(|m| m.promote_on_green).unwrap_or(true);

    // A chat-requested attempt's hand-offs live apart from the blind
    // protocol's: files keyed by the request alone would be shared with a
    // blind attempt of the same model (docs/CHAT-PANE-DESIGN.md §4.1). An
    // answer or a replay creates no directory: the hand-off was posed there,
    // or there is nothing to replay (§R4 CS-12).
    let mut components = vec!["units", unit_id.as_str(), "traces"];
    if requester.is_some() {
        components.push(attempts::CHAT_TRACES);
    }
    let traces = if answer.is_some() || provider_name == "replay" {
        existing_ledger_dir(&ctx, &components).map_err(|e| {
            if answer.is_some() {
                anyhow::Error::new(Error::AnswerRefused {
                    why: format!("no hand-off was posed there: {e}"),
                })
            } else {
                e.context(format!(
                    "unit `{unit_id}` has no recorded attempt to replay: replay verifies \
                     RECORDED attempts from their traces; it never starts a new attempt"
                ))
            }
        })?
    } else {
        safe_ledger_dir(&ctx, &components)?
    };
    let mut resolved = harness_llm::providers::resolve(&provider_name, &traces)?;
    let slot = match answer {
        Some((key, text)) => {
            if resolved.kind != attempts::EXTERNAL_KIND {
                return Err(Error::AnswerRefused {
                    why: format!(
                        "provider `{provider_name}` is of kind `{}`; only an `external` run \
                         files an answer",
                        resolved.kind
                    ),
                }
                .into());
            }
            let slot = std::sync::Arc::new(harness_llm::adapters::AnswerSlot::new(key, text));
            resolved.adapter = Box::new(harness_llm::adapters::TraceAdapter::with_answer(
                &traces,
                slot.clone(),
            ));
            Some(slot)
        }
        None => None,
    };

    let hazards = confirmed_hazards(&ledger, &plan_doc, &facts, &unit_id)?;

    let oracle = harness_oracle::CAbiDifferential;
    let params = harness_llm::migrate::MigrateParams {
        requester: requester.as_deref(),
        answer_key: slot.as_ref().map(|s| s.key.as_str()),
        provider: &resolved,
        model: &model,
        max_tokens,
        max_repairs,
        traces_dir: &traces,
        retry,
        attempt: attempt.as_deref(),
        steer: match (&from, &steer) {
            (Some(from), Some(note)) => Some(harness_llm::SteerArgs { from, note }),
            _ => None,
        },
    };
    let run = harness_llm::migrate::run_migration(
        &params, &oracle, &ctx, &facts, &plan_doc, unit, &hazards,
    );
    // An answer the run never asked for is refused after the fact — the
    // one refusal that cannot come first — and only when the run ended
    // awaiting another request or finished; any other error keeps its own
    // kind (§R4 CE-1). Awaiting: the request it waits on now is reported
    // first, so a client tracks the hand-off it can answer next; finished:
    // the outcome is reported in full first (§R4 CE-4).
    let mut unused = slot.as_ref().filter(|s| !s.used()).and_then(|slot| {
        let why = match &run {
            Ok(o) => format!(
                "the attempt finished ({}) without asking for it",
                o.record.outcome
            ),
            Err(Error::Awaiting { path, .. }) => format!(
                "the attempt asked for another request first ({})",
                report::request_key_of(path).unwrap_or_else(|| path.display().to_string())
            ),
            Err(_) => return None,
        };
        Some(Error::AnswerUnused {
            key: slot.key.clone(),
            why,
        })
    });
    if let Err(Error::Awaiting { path, attempt }) = &run {
        if let Some(unused) = unused.take() {
            report::event(&report::Awaiting {
                k: "awaiting",
                attempt: attempt.as_deref(),
                path: path.display().to_string(),
                resume: resume.clone(),
                args: report::args_without_answer(),
                request_key: report::request_key_of(path),
            });
            return Err(unused.into());
        }
    }
    let outcome = match run {
        Ok(o) => o,
        Err(e @ Error::Awaiting { .. }) => {
            eprintln!("{e:#}");
            eprintln!(
                "migrate: external provider mode — supply the response file under {} and re-run: \
                 {resume}",
                traces.display()
            );
            if let Error::Awaiting { path, attempt } = &e {
                report::event(&report::Awaiting {
                    k: "awaiting",
                    attempt: attempt.as_deref(),
                    path: path.display().to_string(),
                    resume: resume.clone(),
                    args: report::args_without_answer(),
                    request_key: report::request_key_of(path),
                });
            }
            return Err(e.into());
        }
        Err(e) => return Err(e.into()),
    };
    let finish = || -> Result<u8> {
        let record = &outcome.record;
        for (i, t) in record.turns.iter().enumerate() {
            out(format!(
                "migrate: turn {} {} -> {} (tokens in/out: {}/{})",
                i + 1,
                t.kind,
                t.result,
                t.input_tokens
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "?".into()),
                t.output_tokens
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "?".into()),
            ));
        }
        out(format!(
            "migrate: {} attempt {} via `{}` ({}) model `{}` -> {}",
            unit_id,
            record.id,
            record.provider,
            record.provider_kind,
            record.model,
            record.outcome.to_uppercase()
        ));
        if let Some(drifted) = &outcome.drifted {
            out(format!(
                "migrate: verified from its recorded evidence; prompt: {}",
                harness_llm::conformance(drifted)
            ));
        }
        // docs/CLI-HARDENING.md §4: migrate's final verdict — one `check` per
        // check and the `verdict` line — when the last turn was judged by the
        // oracle in this run (a format/truncated/blocked/deny-scan tail stored
        // none; a verification stores nothing).
        if let Some(v) = &outcome.verdict {
            report::verdict(
                &unit_id,
                v,
                &outcome.attempt_dir.join("attempt-verdict.json"),
            );
        }
        let attempt_event = |promoted: bool, promotion: &str| {
            report::event(&report::AttemptEvent {
                k: "attempt",
                unit: &unit_id,
                id: &record.id,
                outcome: &record.outcome,
                provider: &record.provider,
                model: &record.model,
                promoted,
                promotion,
            });
        };
        if record.outcome != "green" {
            attempt_event(record.promoted, "not promoted: not green");
            return Ok(EXIT_ORACLE_RED);
        }

        // Promotion (docs/SCHEMAS.md; docs/CLI-HARDENING.md §2). Precedence:
        // --promote > --no-promote > [llm.migrate] promote_on_green > default;
        // never from a replay run (which writes nothing to the ledger).
        let already_done = matches!(unit.status, UnitStatus::Verified | UnitStatus::Merged);
        let (do_promote, reason) = if resolved.kind == "replay" {
            (false, "replay run")
        } else if promote_flag {
            (true, "--promote")
        } else if no_promote {
            (false, "--no-promote")
        } else if already_done {
            (false, "unit already verified — pass --promote to replace")
        } else if !promote_on_green {
            (false, "promote_on_green = false — run `harness promote`")
        } else {
            (true, "default")
        };
        let (Some(candidate), true) = (outcome.candidate_dir.as_ref(), do_promote) else {
            out(format!(
                "migrate: green attempt recorded; not promoted ({reason})"
            ));
            attempt_event(record.promoted, &format!("not promoted: {reason}"));
            return Ok(0);
        };
        match promote::promote_attempt(
            &ctx,
            &ledger,
            &oracle,
            unit,
            record,
            candidate,
            &outcome.features,
        )? {
            promote::Promotion::Verified => {
                out(format!(
                    "migrate: {unit_id} promoted and verified — status set to verified"
                ));
                attempt_event(true, &format!("promoted: {reason}"));
                Ok(0)
            }
            promote::Promotion::RolledBack => {
                out("migrate: promoted candidate did not verify in place — rolled back".into());
                attempt_event(false, "not promoted: red in place — rolled back");
                Ok(EXIT_ORACLE_RED)
            }
        }
    };
    let code = finish()?;
    match unused {
        Some(unused) => Err(unused.into()),
        None => Ok(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words `sh` makes of `line` (quote removal included).
    fn sh_words(line: &str) -> Vec<String> {
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!(
                "set -- {line}; for a; do printf '%s\\0' \"$a\"; done"
            ))
            .output()
            .expect("run /bin/sh");
        String::from_utf8(out.stdout)
            .expect("utf-8 words")
            .split_terminator('\0')
            .map(str::to_string)
            .collect()
    }

    /// docs/TUI-DESIGN.md §R2 5: the `awaiting` hint, run through a shell,
    /// parses back to the same steer attempt — whatever the note starts
    /// with (a separate `-…` word would be read as a flag, or print help).
    #[test]
    fn the_resume_hint_round_trips_any_note() {
        for note in [
            "- prefer iter() over indexing",
            "-h: keep the wrapping add",
            "--json",
            "Don't index twice; use \"iter()\" & keep $x.",
        ] {
            let args = MigrateArgs {
                requester: None,
                answer: None,
                answer_key: None,
                answer_bytes: None,
                unit: "u-lib".into(),
                target: TargetArg {
                    target: PathBuf::from("/tmp/a target"),
                    tool: Some("t-lz4".into()),
                },
                provider: Some("external".into()),
                model: Some("-m".into()),
                promote: false,
                no_promote: true,
                allow_unsandboxed: false,
                retry: false,
                attempt: None,
                steer: Some(note.into()),
                from: Some("a-0123456789ab".into()),
            };
            let hint = args.resume_command();
            let cli =
                Cli::try_parse_from(sh_words(&hint)).unwrap_or_else(|e| panic!("{hint}\n{e}"));
            let Cmd::Migrate {
                unit,
                target,
                model,
                no_promote,
                steer,
                from,
                ..
            } = cli.cmd
            else {
                panic!("{hint}: not a migrate command");
            };
            assert_eq!(unit, "u-lib");
            assert_eq!(target.target, PathBuf::from("/tmp/a target"));
            assert_eq!(target.tool.as_deref(), Some("t-lz4"), "{hint}");
            assert_eq!(model.as_deref(), Some("-m"));
            assert!(no_promote, "{hint}");
            assert_eq!(steer.as_deref(), Some(note), "{hint}");
            assert_eq!(from.as_deref(), Some("a-0123456789ab"), "{hint}");
            assert!(!cli.json);
        }
    }

    /// The other `awaiting` hints (observe, gen-driver) round-trip the same
    /// way, every run flag kept (§R2 5, fix-pass review).
    #[test]
    fn the_observe_and_gen_driver_hints_round_trip() {
        let hint = observe_resume(&TargetArg {
            target: PathBuf::from("-scratch dir"),
            tool: None,
        });
        let cli = Cli::try_parse_from(sh_words(&hint)).unwrap_or_else(|e| panic!("{hint}\n{e}"));
        let Cmd::Observe { target } = cli.cmd else {
            panic!("{hint}: not observe");
        };
        assert_eq!(target.target, PathBuf::from("-scratch dir"));
        assert_eq!(target.tool, None);
        let args = gen_driver::GenDriverArgs {
            unit: "u-lib".into(),
            target: TargetArg {
                target: PathBuf::from("-t"),
                tool: Some("l-x".into()),
            },
            provider: Some("external".into()),
            model: Some("-m".into()),
            promote: true,
            allow_unsandboxed: true,
            retry: true,
            attempt: Some("d-0123456789ab".into()),
        };
        let hint = args.resume_command();
        let cli = Cli::try_parse_from(sh_words(&hint)).unwrap_or_else(|e| panic!("{hint}\n{e}"));
        let Cmd::GenDriver {
            unit,
            target,
            provider,
            model,
            promote,
            allow_unsandboxed,
            retry,
            attempt,
        } = cli.cmd
        else {
            panic!("{hint}: not gen-driver");
        };
        assert_eq!(
            (
                unit.as_str(),
                target.target,
                provider.as_deref(),
                model.as_deref()
            ),
            ("u-lib", PathBuf::from("-t"), Some("external"), Some("-m"))
        );
        assert_eq!(target.tool.as_deref(), Some("l-x"), "{hint}");
        assert!(promote && allow_unsandboxed && retry, "{hint}");
        assert_eq!(attempt.as_deref(), Some("d-0123456789ab"));
    }
}
