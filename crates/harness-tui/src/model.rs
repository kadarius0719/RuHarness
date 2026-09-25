//! The read model (docs/TUI-DESIGN.md §2): a [`Snapshot`] of a target's
//! ledger, built with harness-core only — the same functions the CLI renders
//! from (`status::unit_report`, `attempts::provenance`,
//! `attempts::current_binding`), so the cockpit never re-implements a hash
//! rule. Rebuilt on demand; never written.

use crate::pairs::{self, FunctionPair};
use harness_core::attempts::{self, AttemptRecord, Authorship, Provenance, Supersession};
use harness_core::error::Error;
use harness_core::facts::Facts;
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
    /// The fact model, when `harness scan` has run.
    pub facts: Option<Facts>,
    /// Its freshness.
    pub facts_state: Option<FactsState>,
    /// The plan's units, in plan order (empty without facts or a plan).
    pub units: Vec<UnitView>,
    /// Why there are no units, when there are none.
    pub note: Option<String>,
}

impl Snapshot {
    /// Read the ledger of the target at `target`.
    pub fn load(target: &Path) -> Result<Snapshot, Error> {
        let ctx = TargetContext::load(target)?;
        let ledger = Ledger::new(&ctx.root);
        let mut snapshot = Snapshot {
            root: ctx.root.clone(),
            facts: None,
            facts_state: None,
            units: Vec::new(),
            note: None,
        };
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
        let plan = match Plan::load(&ledger.plan_path()) {
            Ok(p) => p,
            Err(e) if e.is_not_found() => {
                snapshot.note = Some(NO_PLAN.into());
                snapshot.facts = Some(facts);
                return Ok(snapshot);
            }
            Err(e) => return Err(e),
        };
        for unit in &plan.units {
            snapshot.units.push(unit_view(&ctx, &ledger, &facts, unit)?);
        }
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
) -> Result<UnitView, Error> {
    let report = status::unit_report(ctx, ledger, facts, unit)?;
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
        Authorship::Human(origin) => AuthorshipView::Human(origin.id.clone()),
    };
    let provenance =
        match attempts::provenance(&records, &unit_source, &driver, crate_digest.as_deref()) {
            Provenance::None => ProvenanceView::None,
            Provenance::Pipeline(r) => ProvenanceView::Pipeline(r.id.clone()),
            Provenance::Steered(r) => ProvenanceView::Steered(r.id.clone()),
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

    #[test]
    fn a_c_file_edited_after_the_scan_is_never_sliced() {
        let tmp = std::env::temp_dir().join(format!("harness-tui-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy(&repo().join("targets/zopfli"), &tmp);
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
}
