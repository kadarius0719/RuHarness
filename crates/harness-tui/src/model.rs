//! The read model (docs/TUI-DESIGN.md §2): a [`Snapshot`] of a target's
//! ledger, built with harness-core only — the same functions the CLI renders
//! from (`status::unit_report`, `attempts::provenance`,
//! `attempts::current_binding`), so the cockpit never re-implements a hash
//! rule. Rebuilt on demand; never written.

use crate::pairs::{self, FunctionPair};
use harness_core::attempts::{self, AttemptRecord, Authorship, Provenance, Supersession};
use harness_core::error::Error;
use harness_core::facts::Facts;
use harness_core::features::{FeatureSnapshot, FeaturesNow};
use harness_core::hash;
use harness_core::ledger::Ledger;
use harness_core::plan::{Plan, Unit};
use harness_core::status::{self, UnitReport};
use harness_core::verdict::Verdict;
use harness_core::TargetContext;
use std::path::{Path, PathBuf};

/// How fresh the fact model is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactsState {
    /// Files the scan recorded.
    pub files: usize,
    /// Of them, files whose hash no longer matches the tree.
    pub stale: usize,
    /// Those files (repo-relative, as the facts record them); a missing or
    /// unreadable file counts as stale. Additive (the cockpit's file tree).
    pub stale_paths: Vec<String>,
}

/// `a-13c941dfff95` → `a-13c9`; samples keep their `.rN`.
pub fn short_id(id: &str) -> String {
    let (base, sample) = match id.split_once(".r") {
        Some((b, n)) => (b, format!(".r{n}")),
        None => (id, String::new()),
    };
    let cut: String = base.chars().take(6).collect();
    format!("{cut}{sample}")
}

/// Why Migrate and Modify are greyed when no provider is allowed — the
/// cockpit's own words, which the Speed advice repeats word for word
/// (docs/PERF-DESIGN.md §10 build note 30).
pub const NO_PROVIDER: &str = "no provider is allowed (start with --provider <name>)";

/// Which recorded attempt produced the unit crate (R-5), by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenanceView {
    /// None (on a verified unit: "provenance unknown").
    None,
    /// One unassisted model attempt: the pipeline's.
    Pipeline(String),
    /// Several unassisted model attempts share the crate's digest.
    Ambiguous(Vec<String>),
    /// A steer attempt of model-only lineage (guided by a reviewer's note).
    Steered(String),
    /// A chat-requested unseeded attempt (docs/CHAT-PANE-DESIGN.md §4.2):
    /// never unassisted pipeline output.
    Chat(String),
    /// A hand edit: the matched attempt, and the human attempt at the root
    /// of its seed chain (the same id when it is the human attempt itself).
    Human {
        /// The attempt whose candidate is the crate.
        attempt: String,
        /// The human (override) attempt it descends from.
        origin: String,
    },
}

impl ProvenanceView {
    /// The id of the attempt the crate came from, when there is exactly one.
    pub fn attempt(&self) -> Option<&str> {
        match self {
            ProvenanceView::Pipeline(id)
            | ProvenanceView::Steered(id)
            | ProvenanceView::Chat(id)
            | ProvenanceView::Human { attempt: id, .. } => Some(id),
            ProvenanceView::None | ProvenanceView::Ambiguous(_) => None,
        }
    }
}

/// Who authored an attempt's candidate ([`attempts::authorship`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorshipView {
    /// An unseeded model attempt.
    Pipeline,
    /// A steer attempt of model-only lineage.
    Steered,
    /// An unseeded model attempt a chat asked for.
    Chat,
    /// A human attempt, or a steer attempt descending from the one named.
    Human(String),
}

/// One recorded migrate attempt as the cockpit shows it.
#[derive(Debug, Clone)]
pub struct AttemptView {
    /// The record.
    pub record: AttemptRecord,
    /// Bound to the CURRENT unit source AND driver (the R-5 binding — not
    /// the status summary's source-only flag).
    pub bound: bool,
    /// Its stored verdict (`attempt-verdict.json`), when readable.
    pub verdict: Option<Verdict>,
    /// Its `candidate/` crate, when present.
    pub candidate: Option<PathBuf>,
    /// A human attempt's kept submission (`edit/`, holding `src/logic.rs`
    /// and `src/ffi.rs`) when the judge wrote no candidate.
    pub edit: Option<PathBuf>,
    /// The attempt a `superseded.jsonl` entry names as its successor.
    pub superseded_by: Option<String>,
    /// Who authored it, by its seed lineage.
    pub authorship: AuthorshipView,
}

impl AttemptView {
    /// A labelled human attempt.
    pub fn is_human(&self) -> bool {
        self.record.provider_kind == attempts::HUMAN_KIND
    }

    /// The last turn's result (`""` when there is none).
    pub fn last_result(&self) -> &str {
        self.record.turns.last().map_or("", |t| t.result.as_str())
    }

    /// The Rust this attempt holds: its candidate, else a human attempt's
    /// kept submission.
    pub fn crate_dir(&self) -> Option<&Path> {
        self.candidate.as_deref().or(self.edit.as_deref())
    }
}

/// One plan unit.
#[derive(Debug, Clone)]
pub struct UnitView {
    /// The plan entry.
    pub unit: Unit,
    /// `state status`'s report for it.
    pub report: UnitReport,
    /// Its attempts, in the defined order: bound first, then by base id,
    /// each base before its `.rN` samples.
    pub attempts: Vec<AttemptView>,
    /// Where its crate came from.
    pub provenance: ProvenanceView,
    /// The unit crate (`units/<id>/<rust_crate>/`), when it exists.
    pub crate_dir: Option<PathBuf>,
    /// The committed verdict (`oracle-latest.json`), when readable.
    pub verdict: Option<Verdict>,
    /// The unit crate's content hash (`hash::crate_content_hash`), when it
    /// exists — what provenance and the "known code" test compare.
    /// Additive (the cockpit's file tree).
    pub crate_digest: Option<String>,
}

impl UnitView {
    /// The attempt with `id`.
    pub fn attempt(&self, id: &str) -> Option<&AttemptView> {
        self.attempts.iter().find(|a| a.record.id == id)
    }
}

/// A target's ledger, read.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The target root.
    pub root: PathBuf,
    /// The target's ledger folder (`migration/`, or a mapped tool's
    /// `migration/tools/<id>/`).
    pub ledger_dir: PathBuf,
    /// The fact model, when `harness scan` has run.
    pub facts: Option<Facts>,
    /// Its freshness.
    pub facts_state: Option<FactsState>,
    /// The plan's units, in plan order (empty without facts or a plan).
    pub units: Vec<UnitView>,
    /// Why there are no units, when there are none.
    pub note: Option<String>,
    /// The person's features, read once per load (docs/FEATURES-DESIGN.md
    /// §2.2, §8.1): a bad file is a value, never a failed read.
    pub features: FeatureSnapshot,
    /// Today's feature digests (`None` without a features file).
    pub features_now: Option<FeaturesNow>,
    /// `[target]` as configured: the run name and the form — the folder
    /// (`source_dir`) or the listed files.
    pub target: harness_core::config::TargetSection,
    /// The file name the program runs under in a scenario.
    pub program_name: String,
    /// perf's files and today's inputs (docs/PERF-DESIGN.md §3.11).
    pub perf: crate::perfread::PerfRead,
    /// A mapped tool's "project changed" notice, in words
    /// (docs/PROJECT-MAP-DESIGN.md §3.7): a notice, never staleness.
    pub project_notice: Option<String>,
}

impl Snapshot {
    /// Read the ledger of the target at `target`.
    pub fn load(target: &Path) -> Result<Snapshot, Error> {
        Snapshot::open(target, None)
    }

    /// The target's ledger.
    pub fn ledger(&self) -> Ledger {
        Ledger::at(&self.root, &self.ledger_dir)
    }

    /// The ledger folder relative to the root, `/`-joined (`migration`, or
    /// `migration/tools/<id>`), for the words that name its files.
    pub fn ledger_rel(&self) -> String {
        match self.ledger_dir.strip_prefix(&self.root) {
            Ok(rel) => rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => harness_core::ledger::MIGRATION_DIR.to_string(),
        }
    }

    /// Read the ledger of the target `--target <target> [--tool <tool>]`
    /// names, either form.
    pub fn open(target: &Path, tool: Option<&str>) -> Result<Snapshot, Error> {
        let ctx = TargetContext::open(target, tool)?;
        let ledger = Ledger::of(&ctx);
        let mut snapshot = Snapshot {
            root: ctx.root.clone(),
            ledger_dir: ctx.ledger.clone(),
            facts: None,
            facts_state: None,
            units: Vec::new(),
            note: None,
            features: FeatureSnapshot::load(&ctx),
            features_now: None,
            target: ctx.config.target.clone(),
            program_name: harness_core::features::program_name(&ctx.config),
            perf: crate::perfread::PerfRead::default(),
            project_notice: harness_core::ledger::project_changed_notice(&ctx),
        };
        // perf's files: the live lock holder read here (as `unit_report`
        // does) — while a perf run holds it no input is hashed.
        let holder = status::live_holder(&ledger).ok().flatten();
        let holder_command = holder.as_ref().map(|h| h.command.clone());
        let perf_units: Vec<(String, Option<String>)> = Plan::load(&ledger.plan_path())
            .map(|p| {
                p.units
                    .iter()
                    .map(|u| {
                        (
                            u.id.clone(),
                            u.oracle_param_str("rust_crate").map(str::to_string),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        snapshot.perf = crate::perfread::read(&ledger, &perf_units, holder_command.as_deref());
        let facts = match Facts::load(&ledger.facts_path()) {
            Ok(f) => f,
            Err(e) if e.is_not_found() => {
                snapshot.note = Some("no facts — run `harness scan`".into());
                return Ok(snapshot);
            }
            Err(e) => return Err(e),
        };
        let stale_paths: Vec<String> = facts
            .files
            .iter()
            .filter(|f| {
                hash::file_hash(&ctx.root.join(&f.path))
                    .map(|h| h != f.hash)
                    .unwrap_or(true)
            })
            .map(|f| f.path.clone())
            .collect();
        snapshot.facts_state = Some(FactsState {
            files: facts.files.len(),
            stale: stale_paths.len(),
            stale_paths,
        });
        let has_workloads = !matches!(
            snapshot.perf.workloads,
            Ok(harness_core::perf::workloads::WorkloadsState::NoFile)
        );
        let plan = match Plan::load(&ledger.plan_path()) {
            Ok(p) => p,
            Err(e) if e.is_not_found() => {
                // The C alone is measured from day one, before a plan
                // (docs/PERF-DESIGN.md §3.6): its rows are judged too.
                if has_workloads {
                    snapshot.perf.program_now =
                        Some(harness_core::features::program_digest_now(&ctx, &facts));
                }
                snapshot.note = Some(NO_PLAN.into());
                snapshot.facts = Some(facts);
                return Ok(snapshot);
            }
            Err(e) => return Err(e),
        };
        let now = FeaturesNow::compute(&ctx, &facts, &snapshot.features);
        for unit in &plan.units {
            snapshot
                .units
                .push(unit_view(&ctx, &ledger, &facts, unit, now.as_ref())?);
        }
        // The program's digest for perf's currency: the features' when
        // computed, else hashed here — only with a workloads file.
        if has_workloads {
            snapshot.perf.program_now = Some(match &now {
                Some(n) => n.program.clone(),
                None => harness_core::features::program_digest_now(&ctx, &facts),
            });
        }
        snapshot.features_now = now;
        snapshot.facts = Some(facts);
        Ok(snapshot)
    }

    /// The unit with `id`.
    pub fn unit(&self, id: &str) -> Option<&UnitView> {
        self.units.iter().find(|u| u.unit.id == id)
    }

    /// The function pairs of `unit` against `crate_dir` (the unit crate, or
    /// an attempt's `candidate/`).
    pub fn pairs(&self, unit: &UnitView, crate_dir: Option<&Path>) -> Vec<FunctionPair> {
        match &self.facts {
            Some(facts) => pairs::pairs(&self.root, facts, &unit.unit, crate_dir),
            None => Vec::new(),
        }
    }
}

/// [`Snapshot::note`] when the plan file does not exist.
pub const NO_PLAN: &str = "no plan — run `harness plan`";

/// The binding of inputs that could not be read: no record carries it.
const UNREADABLE: &str = "blake3:unreadable";

/// `(base id, sample number)`: `<base>` is sample 1, `<base>.r<N>` sample N.
fn sample_key(id: &str) -> (&str, u32) {
    match id.rsplit_once(".r") {
        Some((base, n)) => match n.parse::<u32>() {
            Ok(n) if n >= 2 => (base, n),
            _ => (id, 1),
        },
        None => (id, 1),
    }
}

fn unit_view(
    ctx: &TargetContext,
    ledger: &Ledger,
    facts: &Facts,
    unit: &Unit,
    features: Option<&FeaturesNow>,
) -> Result<UnitView, Error> {
    let report = status::unit_report(ctx, ledger, facts, unit, features)?;
    let records = attempts::load_unit_attempts(ledger, &unit.id)?;
    // A source or driver deleted since the scan binds nothing — as `state
    // status` reads it ("unreadable") — rather than making the whole ledger
    // unreadable: the cockpit shows the file missing. (With the source
    // unbound no attempt is bound whatever the driver's hash, so both are
    // set alike.)
    let (unit_source, driver) = match attempts::current_binding(ctx, facts, unit) {
        Ok(binding) => binding,
        // Missing inputs bind nothing (no record carries this binding);
        // any other error still fails the read.
        Err(e) if e.is_not_found() => (UNREADABLE.into(), UNREADABLE.into()),
        Err(e) => return Err(e),
    };
    let crate_digest = attempts::unit_crate_digest(ledger, unit)?;
    let authorship = |r: &AttemptRecord| match attempts::authorship(&records, r) {
        Authorship::Pipeline => AuthorshipView::Pipeline,
        Authorship::Steered => AuthorshipView::Steered,
        Authorship::Chat => AuthorshipView::Chat,
        Authorship::Human(origin) => AuthorshipView::Human(origin.id.clone()),
    };
    let provenance =
        match attempts::provenance(&records, &unit_source, &driver, crate_digest.as_deref()) {
            Provenance::None => ProvenanceView::None,
            Provenance::Pipeline(r) => ProvenanceView::Pipeline(r.id.clone()),
            Provenance::Steered(r) => ProvenanceView::Steered(r.id.clone()),
            Provenance::Chat(r) => ProvenanceView::Chat(r.id.clone()),
            Provenance::Human(r) => ProvenanceView::Human {
                attempt: r.id.clone(),
                origin: match authorship(r) {
                    AuthorshipView::Human(origin) => origin,
                    _ => r.id.clone(),
                },
            },
            Provenance::Ambiguous(rs) => {
                ProvenanceView::Ambiguous(rs.iter().map(|r| r.id.clone()).collect())
            }
        };
    let authored: Vec<AuthorshipView> = records.iter().map(authorship).collect();
    // A malformed superseded.jsonl is shown as absent here; `bench check`
    // reports it.
    let supersessions: Vec<Supersession> =
        attempts::load_supersessions(ledger, &unit.id).unwrap_or_default();
    let mut views: Vec<AttemptView> = records
        .into_iter()
        .zip(authored)
        .map(|(record, authorship)| {
            let dir = attempts::attempt_dir(ledger, &unit.id, &record.id);
            let candidate = dir.join("candidate");
            let edit = dir.join(attempts::HUMAN_EDIT_DIR);
            AttemptView {
                bound: record.unit_source == unit_source && record.driver == driver,
                verdict: Verdict::load(&dir.join("attempt-verdict.json")).ok(),
                candidate: candidate.join("Cargo.toml").is_file().then_some(candidate),
                edit: (record.provider_kind == attempts::HUMAN_KIND
                    && edit.join("src/logic.rs").is_file())
                .then_some(edit),
                superseded_by: supersessions
                    .iter()
                    .find(|s| s.stage == "migrate" && s.attempt == record.id)
                    .map(|s| s.superseded_by.clone()),
                authorship,
                record,
            }
        })
        .collect();
    views.sort_by(|a, b| {
        let (ab, an) = sample_key(&a.record.id);
        let (bb, bn) = sample_key(&b.record.id);
        (!a.bound, ab, an).cmp(&(!b.bound, bb, bn))
    });
    let crate_dir = unit
        .oracle_param_str("rust_crate")
        .map(|name| ledger.unit_dir(&unit.id).join(name))
        .filter(|dir| dir.join("Cargo.toml").is_file());
    Ok(UnitView {
        unit: unit.clone(),
        report,
        attempts: views,
        provenance,
        crate_dir,
        verdict: Verdict::load(&ledger.verdict_latest_path(&unit.id)).ok(),
        crate_digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairs::{CSide, RustNote};

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn the_read_scalefactors_case_reads_as_the_ledger_says() {
        let root =
            repo().join("targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib");
        let snap = Snapshot::load(&root).unwrap();
        assert_eq!(snap.facts_state.as_ref().unwrap().stale, 0);
        let unit = snap.unit("u-lib").expect("u-lib");
        assert_eq!(
            unit.provenance,
            ProvenanceView::Pipeline("a-13c941dfff95".into())
        );
        let old = unit
            .attempt("a-28d8ddc411f9")
            .expect("the superseded attempt");
        assert_eq!(old.superseded_by.as_deref(), Some("a-13c941dfff95"));
        // Bound attempts first.
        let first_unbound = unit.attempts.iter().position(|a| !a.bound);
        if let Some(i) = first_unbound {
            assert!(unit.attempts[i..].iter().all(|a| !a.bound));
        }
        let pairs = snap.pairs(unit, unit.crate_dir.as_deref());
        let p = pairs
            .iter()
            .find(|p| p.symbol == "read_scalefactors")
            .expect("the exported symbol");
        assert!(matches!(&p.c, CSide::Source(s) if s.lines[0].contains("read_scalefactors")));
        assert!(p.rust.shim.as_ref().unwrap().file.ends_with("ffi.rs"));
        assert!(p.rust.logic.is_some(), "{:?}", p.rust);
    }

    #[test]
    fn the_zopfli_m0_crate_has_an_inline_ffi_module_and_no_provenance() {
        let root = repo().join("targets/zopfli");
        let snap = Snapshot::load(&root).unwrap();
        let unit = snap.unit("u001-katajainen").expect("u001");
        assert_eq!(unit.provenance, ProvenanceView::None);
        let pairs = snap.pairs(unit, unit.crate_dir.as_deref());
        let p = pairs
            .iter()
            .find(|p| p.symbol == "ZopfliLengthLimitedCodeLengths")
            .expect("the exported symbol");
        assert_eq!(p.rust.shim.as_ref().unwrap().file, "src/lib.rs");
        assert_eq!(
            p.rust.logic.as_ref().unwrap().name,
            "length_limited_code_lengths"
        );
        // An internal symbol pairs with the function of its name, if any.
        for p in pairs.iter().filter(|p| !p.public) {
            assert!(p.rust.shim.is_none());
            assert!(p.rust.logic.is_some() || p.rust.note == Some(RustNote::NotFound));
        }
    }

    /// The read model refuses a ledger made elsewhere with the CLI's
    /// sentence (docs/PROJECT-MAP-DESIGN.md §3.7), and so does a read.
    #[test]
    fn a_ledger_made_elsewhere_is_refused_by_the_read_model() {
        let tmp =
            std::env::temp_dir().join(format!("harness-tui-elsewhere-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy(&repo().join("targets/zopfli"), &tmp);
        harness_core::adopt::testing::adoption_file();
        let err = Snapshot::load(&tmp).unwrap_err();
        let head = format!(
            "{}: this folder already holds migration results made elsewhere (11 units, 1 \
             verified)",
            tmp.canonicalize().unwrap().display()
        );
        assert_eq!(
            err.to_string(),
            format!("{head}: to trust them here, add `--adopt` once")
        );
        // The cockpit names only its own way.
        assert_eq!(
            crate::load::read(&tmp).unwrap_err(),
            format!(
                "{head}: to trust them here, start the cockpit in a terminal and answer its \
                 question"
            )
        );
        harness_core::adopt::testing::adopt(&tmp);
        Snapshot::load(&tmp).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_c_file_edited_after_the_scan_is_never_sliced() {
        let tmp = std::env::temp_dir().join(format!("harness-tui-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy(&repo().join("targets/zopfli"), &tmp);
        harness_core::adopt::testing::adopt(&tmp);
        let snap = Snapshot::load(&tmp).unwrap();
        let unit = snap.unit("u001-katajainen").unwrap().clone();
        let before = snap.pairs(&unit, None);
        let file = match &before[0].c {
            CSide::Source(s) => s.file.clone(),
            other => panic!("{other:?}"),
        };
        // Lines inserted above every function: the span would now be wrong.
        let path = tmp.join(&file);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("/* a */\n/* b */\n{text}")).unwrap();
        let snap = Snapshot::load(&tmp).unwrap();
        assert!(snap.facts_state.as_ref().unwrap().stale >= 1);
        let after = snap.pairs(&unit, None);
        assert_eq!(after[0].c, CSide::StaleFacts { file });
        assert_eq!(after[0].rust.note, Some(RustNote::NoCrate));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn copy(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name();
            if name == "build" || name == "target" {
                continue;
            }
            let (from, to) = (entry.path(), dst.join(&name));
            if from.is_dir() {
                copy(&from, &to);
            } else {
                std::fs::copy(&from, &to).unwrap();
            }
        }
    }

    /// A source deleted since the scan binds nothing — it never makes the
    /// ledger unreadable.
    #[test]
    fn a_deleted_source_binds_nothing_and_the_ledger_still_reads() {
        let tmp = std::env::temp_dir().join(format!("harness-tui-deleted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy(
            &repo().join("targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib"),
            &tmp,
        );
        std::fs::remove_file(tmp.join("test_case/include/lib.h")).unwrap();
        harness_core::adopt::testing::adopt(&tmp);
        let snap = Snapshot::load(&tmp).expect("the ledger still reads");
        let unit = snap.unit("u-lib").unwrap();
        assert!(unit.attempts.iter().all(|a| !a.bound));
        assert_eq!(unit.provenance, ProvenanceView::None);
        assert!(snap
            .facts_state
            .as_ref()
            .unwrap()
            .stale_paths
            .contains(&"test_case/include/lib.h".to_string()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Review ENG-10: only a MISSING input binds nothing; any other read
    /// error still fails the read (the cockpit keeps its last snapshot).
    #[test]
    fn an_unreadable_source_still_fails_the_read() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("harness-tui-eacces-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy(
            &repo().join("targets/tractor/cases/Hidden-Tests/B01_organic/read_scalefactors_lib"),
            &tmp,
        );
        let h = tmp.join("test_case/include/lib.h");
        std::fs::set_permissions(&h, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = Snapshot::load(&tmp);
        std::fs::set_permissions(&h, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(result.is_err(), "a permission error is not a missing file");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn sample_keys_order_bases_before_their_samples() {
        assert_eq!(sample_key("a-1"), ("a-1", 1));
        assert_eq!(sample_key("a-1.r2"), ("a-1", 2));
        assert_eq!(sample_key("a-1.rx"), ("a-1.rx", 1));
    }

    /// A mapped tool's "project changed" notice (docs/PROJECT-MAP-DESIGN.md
    /// §3.7): computed from the map file's digests, absent on a
    /// hand-written tool with no `map`.
    #[test]
    fn the_read_model_carries_the_project_changed_notice() {
        let tmp = std::env::temp_dir().join(format!(
            "harness-tui-notice-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let tmp = tmp.canonicalize().unwrap();
        std::fs::write(tmp.join("a.c"), "int a(void) { return 1; }\n").unwrap();
        let tool = |id: &str, map: &str| {
            let dir = harness_core::config::tool_dir(&tmp, id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("harness.toml"),
                format!(
                    "schema_version = 2\n[target]\nname = \"a\"\nfiles = [{{ path = \"a.c\" }}]\n\
                     configuration = {{ name = \"plain\", from = \"stated\", flags = [] }}\n{map}"
                ),
            )
            .unwrap();
        };
        let digest = |c: char| format!("blake3:{}", c.to_string().repeat(64));
        tool(
            "t-a",
            &format!(
                "map = {{ root_hash = \"{}\", inputs_hash = \"{}\" }}\n",
                digest('a'),
                digest('b')
            ),
        );
        tool("t-hand", "");
        harness_core::adopt::testing::adopt(&tmp);
        let notice = |id: &str| Snapshot::open(&tmp, Some(id)).unwrap().project_notice;
        assert!(notice("t-a").unwrap().starts_with("no map written yet"));
        assert_eq!(notice("t-hand"), None);
        let map = |root: char| {
            std::fs::create_dir_all(tmp.join("migration/map")).unwrap();
            std::fs::write(
                tmp.join(harness_core::ledger::PROJECT_MAP_FILE),
                format!(
                    "{{\"root_hash\": \"{}\", \"inputs_hash\": \"{}\"}}\n",
                    digest(root),
                    digest('b')
                ),
            )
            .unwrap();
        };
        map('a');
        assert_eq!(notice("t-a"), None);
        map('c');
        assert_eq!(
            notice("t-a").as_deref(),
            Some(
                "the project changed since this tool was accepted: run `harness project map`, \
                 then `accept` again"
            )
        );
        assert_eq!(notice("t-hand"), None);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
