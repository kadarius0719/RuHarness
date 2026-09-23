//! The trajectory engine the executor's stages share (docs/M4-DESIGN.md
//! §5.1): pose one unit to a model as a first turn plus up to
//! `max_repairs` STATELESS repair turns, judge every candidate, and journal
//! the trajectory in an attempts ledger. What differs between the stages —
//! prompts, emission spec, candidate writing and judging, the first turn's
//! kind, the attempts subdirectory, the id derivation and the record's
//! `stage` — is behind the private [`Stage`] trait; everything else here is
//! stage-blind.
//!
//! # Determinism
//!
//! Every step is a pure function of the target tree, the provider identity
//! and the model's replies: prompts contain no clock, counter, or absolute
//! path (tool output quoted as evidence has this machine's paths scrubbed:
//! candidate, target, toolchain, home and temp dirs), and the attempt id is
//! content-derived.
//!
//! # Re-runs never destroy evidence
//!
//! A FINISHED attempt (`outcome != "in-progress"`) is evidence. Under no
//! provider is its record, candidate, or judge record reset, deleted, or
//! rewritten with different content. What a re-run of the same
//! content-derived id does depends on the provider:
//!
//! - **trace-backed hand-off (`external`)**: an `in-progress` attempt is
//!   RESUMED in its directory — the normal rhythm of this provider, whose
//!   adapter errors with "awaiting response" (propagated unchanged) until
//!   the reply file exists; the earlier turns replay from their traces. A
//!   finished attempt is VERIFIED instead (see `replay` below): the
//!   deterministic trajectory is re-run in a scratch dir, must reproduce the
//!   record, and the on-disk record and candidate are returned untouched.
//! - **live providers**: every call is a fresh sample, so a finished attempt
//!   is refused ("already finished … pass --retry") before anything is
//!   touched or sent. With [`MigrateParams::retry`] the new sample gets its
//!   own id `<base-id>.r<N>` (N = 2, 3, … = 1 + the number of sample dirs
//!   that exist for the base id) and its own directory. Live calls are
//!   recorded under `<traces_dir>/<attempt-id>/` — per sample, never the
//!   shared root — so samples cannot overwrite each other's traces. An
//!   `in-progress` live attempt is what a crashed or interrupted run leaves
//!   behind: it is resumed by STARTING OVER in the same directory (its
//!   leftover candidate and judge record are dropped, every call is made
//!   again; trace files of the interrupted run stay where they are and are
//!   overwritten only where a request key recurs).
//! - **`replay`** verifies a recorded attempt and writes nothing under the
//!   attempts ledger: the record whose first-turn `request_key` equals the
//!   one computed from the tree is re-run from its traces (the sample's own
//!   trace dir when it has one, else the root) in a scratch dir, and every
//!   turn's `request_key`, `response_hash` and `result`, the
//!   `candidate_digest` and the `outcome` must match the record.
//!
//! # Trust
//!
//! - C source, `facts.jsonl`, `plan.toml` and hazard records are hostile
//!   input. Source files are read only through clean relative paths that
//!   still resolve inside the target's `[target] source_dir` (R2: nothing
//!   else — held-out vectors above all — can reach a provider), and travel
//!   JSON-string-encoded with `<` escaped inside nonce-delimited blocks. ABI
//!   lines are reduced to one printable line each.
//! - Model output is untrusted code, written only through the stage's
//!   closed emission spec into a FRESH candidate directory, and every
//!   directory written to is checked, level by level, to be the real
//!   `migration/units/<id>/…` directory rather than a symlink out of it.
//! - Tool output fed back as repair evidence is bounded, reduced to
//!   printable ASCII, and quoted line by line behind a `| ` prefix, so it
//!   can never imitate a prompt section.

use crate::adapters::TraceAdapter;
use crate::emission::{self, FileSpec, ParsedEmission};
use crate::migrate::MigrateParams;
use crate::providers::{
    checked_complete, is_context_error, preflight, EnvLookup, ResolvedProvider,
};
use crate::triage::{encode_slice, is_clean_relative_path};
use harness_core::attempts::{AttemptRecord, Turn, ATTEMPT_SCHEMA_NAME, ATTEMPT_SCHEMA_VERSION};
use harness_core::error::Error;
use harness_core::facts::Facts;
use harness_core::hash;
use harness_core::ledger::Ledger;
use harness_core::plan::{is_clean_segment, Unit};
use harness_core::traits::CompletionRequest;
use std::path::{Path, PathBuf};

/// Provider kind that verifies a recorded attempt without touching the ledger.
pub(crate) const REPLAY_KIND: &str = "replay";
/// `outcome` of an attempt whose trajectory has not ended.
pub(crate) const IN_PROGRESS: &str = "in-progress";
/// Most emission notes passed on to the model in one repair turn.
const MAX_EMISSION_NOTES: usize = 5;
/// Bound on each emission note, in bytes.
const EMISSION_NOTE_MAX_BYTES: usize = 200;
/// Bound on quoted build output, in bytes.
pub(crate) const BUILD_EVIDENCE_MAX_BYTES: usize = 6 * 1024;
/// Bound on any other quoted check detail, in bytes.
pub(crate) const DETAIL_MAX_BYTES: usize = 1024;
/// Most failed checks quoted in one evidence section.
pub(crate) const MAX_FAILED_CHECKS: usize = 8;
/// Bound on each `[ABI CONTRACT]` line, in bytes.
pub(crate) const CONTRACT_LINE_MAX_BYTES: usize = 512;
/// Judge records a run leaves in an attempt dir besides `candidate/`: the
/// migrate verdict and the driver validation. Both are regenerated by the
/// trajectory, so an unfinished attempt started over drops them.
const JUDGE_RECORDS: [&str; 2] = ["attempt-verdict.json", "validation.json"];

/// Why the files of the most recent parseable reply were not green.
pub(crate) struct Failure {
    /// Failure class = the turn's `result`.
    pub(crate) class: &'static str,
    /// The `[FAILURE CLASS]` explanation.
    pub(crate) explanation: &'static str,
    /// The `[EVIDENCE]` text (emission notes are appended by the engine).
    pub(crate) evidence: String,
}

/// What a stage's judge made of one parsed reply.
pub(crate) struct Judged {
    /// Whether a candidate was written under the work dir.
    pub(crate) wrote_candidate: bool,
    /// `None` = green.
    pub(crate) failure: Option<Failure>,
}

/// Where a run writes, handed to [`Stage::judge`].
pub(crate) struct RunCtx<'a> {
    /// The attempt dir — or the verification scratch dir.
    pub(crate) work_dir: &'a Path,
    /// `work_dir` relative to the unit dir, `/`-joined.
    pub(crate) work_rel: String,
    /// Verifying a recorded attempt: the judge's record is NOT journaled.
    pub(crate) verifying: bool,
    /// Absolute paths replaced in quoted tool output ([`scrub_list`]).
    pub(crate) scrub: &'a [(String, String)],
}

/// A stage's fixed texts and names. Every string is harness-authored.
pub(crate) struct StageTexts {
    /// `Turn.kind` of the first turn (`translate`, `generate`).
    pub(crate) first_kind: &'static str,
    /// Attempts subdirectory of the unit dir (`attempts`, `driver-attempts`).
    pub(crate) attempts_subdir: &'static str,
    /// `AttemptRecord.stage` (`None` = migrate: the field is omitted).
    pub(crate) stage: Option<&'static str>,
    /// The fixed system prompt of every turn.
    pub(crate) system: &'static str,
    /// The emission contract.
    pub(crate) spec: &'static FileSpec,
    /// The `[TASK]` of the first turn.
    pub(crate) first_task: &'static str,
    /// The `[TASK]` of a repair turn.
    pub(crate) repair_task: &'static str,
    /// Name of the section showing the current candidate (`CURRENT RUST`).
    pub(crate) current_section: &'static str,
    /// That section's body before any reply parsed.
    pub(crate) no_current: &'static str,
    /// Lead-in for a format failure after an earlier parseable reply,
    /// completed by `failed with class …`.
    pub(crate) earlier: &'static str,
    /// What the first prompt is a function of (replay error messages).
    pub(crate) prompt_inputs: &'static str,
}

/// What varies between the executor's stages (private seam: docs/M4-DESIGN.md
/// §5.1, R11). Implementations own their judge and whatever it needs.
pub(crate) trait Stage {
    /// The stage's fixed texts and names.
    fn texts(&self) -> &StageTexts;

    /// Content-derived base id of an attempt whose first request has key
    /// `first_key` (migrate ids are FROZEN; driver ids mix in the stage).
    fn attempt_id(
        &self,
        unit: &str,
        unit_source: &str,
        driver: &str,
        provider_kind: &str,
        model: &str,
        first_key: &str,
    ) -> String;

    /// Judge one parsed reply (`files` in spec order): pre-checks, writing
    /// the candidate under `ctx.work_dir`, the judge itself, journaling its
    /// record (unless `ctx.verifying`), and setting the record's
    /// `candidate_digest` and `toolchain` when a judge ran.
    fn judge(
        &self,
        ctx: &RunCtx,
        files: &[String],
        record: &mut AttemptRecord,
    ) -> Result<Judged, Error>;

    /// The candidate a run wrote under `work_dir`, as reported to callers.
    fn candidate_path(&self, work_dir: &Path) -> PathBuf;

    /// Digest of the candidate at `path`, as `candidate_digest` records it.
    fn candidate_digest(&self, path: &Path) -> Result<String, Error>;
}

/// What one stage run produced (see [`crate::migrate::MigrationOutcome`]).
pub(crate) struct Outcome {
    /// The final attempt record.
    pub(crate) record: AttemptRecord,
    /// `migration/units/<unit>/<attempts subdir>/<attempt-id>/`.
    pub(crate) attempt_dir: PathBuf,
    /// The last candidate written, or the verified original.
    pub(crate) candidate: Option<PathBuf>,
}

/// Everything one stage run is bound to.
pub(crate) struct Job<'a> {
    pub(crate) params: &'a MigrateParams<'a>,
    pub(crate) stage: &'a dyn Stage,
    pub(crate) unit: &'a Unit,
    /// Rooted at the canonical target root.
    pub(crate) ledger: &'a Ledger,
    /// The canonical target root.
    pub(crate) root: &'a Path,
    /// The target root as the caller gave it (scrubbed too).
    pub(crate) target_root: &'a Path,
    /// The sections every turn shares verbatim.
    pub(crate) pinned: &'a str,
    /// File-set digest of the sources in `pinned`.
    pub(crate) unit_source: String,
    /// Digest of the differential driver (`""` for the driver stage).
    pub(crate) driver: String,
}

impl<'a> Job<'a> {
    /// Run the stage for the job's unit — see the module docs for what a
    /// re-run of an existing attempt does.
    pub(crate) fn run(&self) -> Result<Outcome, Error> {
        let params = self.params;
        let provider = params.provider;
        let texts = self.stage.texts();
        let first = self.request(format!("{}\n[TASK]\n{}\n", self.pinned, texts.first_task));
        // Before any record exists; `checked_complete` repeats it per call.
        preflight(provider, &first)?;
        let first_key = TraceAdapter::request_key(&first)?;

        if provider.kind == REPLAY_KIND {
            let recorded = self.find_recorded(&first_key)?;
            // A live sample's traces live in its own dir; everything recorded
            // before per-sample dirs existed, and every hand-off, in the root
            // the replay adapter was constructed with.
            let sample_traces = params.traces_dir.join(&recorded.id);
            let sample_provider = sample_traces.is_dir().then(|| ResolvedProvider {
                adapter: Box::new(TraceAdapter::new(&sample_traces, false)),
                profile: provider.profile.clone(),
                kind: provider.kind.clone(),
                context_tokens: provider.context_tokens,
                live: false,
            });
            self.verify_recorded(
                &recorded,
                sample_provider.as_ref().unwrap_or(provider),
                first,
            )?;
            return Ok(Outcome {
                attempt_dir: self.attempts_dir().join(&recorded.id),
                record: recorded,
                candidate: None,
            });
        }

        let base_id = self.stage.attempt_id(
            &self.unit.id,
            &self.unit_source,
            &self.driver,
            &provider.kind,
            params.model,
            &first_key,
        );
        let id = if provider.live {
            live_sample_id(&self.attempts_dir(), &base_id, params.retry)?
        } else {
            let attempt_dir = self.attempts_dir().join(&base_id);
            match load_record(&attempt_dir, &base_id)? {
                Some(finished) if finished.outcome != IN_PROGRESS => {
                    // Evidence is never rewritten: the deterministic
                    // trajectory is re-run in scratch and must reproduce it.
                    self.verify_recorded(&finished, provider, first)?;
                    let candidate = self.recorded_candidate(&attempt_dir, &finished)?;
                    return Ok(Outcome {
                        record: finished,
                        attempt_dir,
                        candidate,
                    });
                }
                _ => base_id,
            }
        };

        // From here on the attempt dir is a new or a never-finished one.
        let mut record = AttemptRecord {
            schema: ATTEMPT_SCHEMA_NAME.to_string(),
            schema_version: ATTEMPT_SCHEMA_VERSION,
            id: id.clone(),
            unit: self.unit.id.clone(),
            stage: texts.stage.map(str::to_string),
            provider: provider.profile.clone(),
            provider_kind: provider.kind.clone(),
            model: params.model.to_string(),
            prompt_digest: prompt_digest(&first),
            unit_source: self.unit_source.clone(),
            driver: self.driver.clone(),
            toolchain: Vec::new(),
            outcome: IN_PROGRESS.to_string(),
            turns: Vec::new(),
            candidate_digest: String::new(),
            promoted: false,
        };
        let work_rel = vec![texts.attempts_subdir.to_string(), id.clone()];
        let work_dir = prepare_dir(self.ledger, &self.unit.id, &work_rel)?;
        reset_unfinished(&work_dir, &id)?;
        record.store(&work_dir)?;

        let max_turns = usize::try_from(params.max_repairs)
            .unwrap_or(usize::MAX)
            .saturating_add(1);
        let run = self.run_in(
            provider,
            &work_dir,
            &work_rel,
            None,
            provider.live.then(|| params.traces_dir.join(&id)),
            max_turns,
        );
        let wrote_candidate = run.drive(&mut record, first)?;

        Ok(Outcome {
            record,
            candidate: wrote_candidate.then(|| self.stage.candidate_path(&work_dir)),
            attempt_dir: work_dir,
        })
    }

    /// `migration/units/<unit>/<attempts subdir>/`.
    fn attempts_dir(&self) -> PathBuf {
        self.ledger
            .unit_dir(&self.unit.id)
            .join(self.stage.texts().attempts_subdir)
    }

    /// A request for this run's model and budget under the stage's fixed
    /// system prompt.
    fn request(&self, user: String) -> CompletionRequest {
        CompletionRequest {
            model: self.params.model.to_string(),
            system: self.stage.texts().system.to_string(),
            user,
            max_tokens: self.params.max_tokens,
        }
    }

    /// A run over `work_dir` (= the unit dir joined with `work_rel`).
    fn run_in(
        &'a self,
        provider: &'a ResolvedProvider,
        work_dir: &'a Path,
        work_rel: &[String],
        verifying: Option<&'a AttemptRecord>,
        record_traces: Option<PathBuf>,
        max_turns: usize,
    ) -> Run<'a> {
        Run {
            job: self,
            provider,
            work_dir,
            work_rel: work_rel.join("/"),
            verifying,
            record_traces,
            max_turns,
            scrub: {
                let mut scrub = scrub_list(
                    &work_dir.join("candidate"),
                    self.root,
                    self.target_root,
                    &|name| std::env::var_os(name),
                    &std::env::temp_dir(),
                );
                // Verifying runs in `.replay-<id>/`, the recorded run ran in
                // `<attempts subdir>/<id>/`: judged evidence quoting the
                // candidate's path (compiler output does) must read as the
                // recorded run's did — BEFORE it is bounded, since the two
                // paths differ in length and truncation would differ too.
                if let Some(recorded) = verifying {
                    let unit = &self.unit.id;
                    scrub.insert(
                        0,
                        (
                            format!("/migration/units/{unit}/{}/", work_rel.join("/")),
                            format!(
                                "/migration/units/{unit}/{}/{}/",
                                self.stage.texts().attempts_subdir,
                                recorded.id
                            ),
                        ),
                    );
                }
                scrub
            },
        }
    }

    /// Verify `recorded`: re-run its trajectory, completions coming from
    /// `provider`, against a scratch candidate — writing NOTHING under the
    /// attempts ledger — and require that it reproduces the record. The
    /// scratch dir sits in the unit dir because the judges only accept
    /// candidates there; it is removed again whatever happens.
    ///
    /// The turn budget is the record's, not `max_repairs`: the budget that
    /// ended a recorded `red`/`format` trajectory is a property of that run
    /// (`attempt.json` does not store it), and any trajectory that ended by
    /// itself reproduces under every budget that reaches its last turn.
    fn verify_recorded(
        &self,
        recorded: &AttemptRecord,
        provider: &ResolvedProvider,
        first: CompletionRequest,
    ) -> Result<(), Error> {
        if recorded.outcome == IN_PROGRESS {
            return Err(Error::Invariant(format!(
                "attempt {} is still in progress: only a finished attempt can be verified — \
                 finish it with the provider that started it (`{}`)",
                recorded.id,
                printable(&recorded.provider, 64)
            )));
        }
        let scratch_rel = vec![format!(".replay-{}", recorded.id)];
        // A crashed verification may have left its scratch behind. The unit
        // dir is verified first, so not even this removal goes through a
        // symlink.
        let unit_dir = prepare_dir(self.ledger, &self.unit.id, &[])?;
        remove_path(&unit_dir.join(&scratch_rel[0]))?;
        let scratch = prepare_dir(self.ledger, &self.unit.id, &scratch_rel)?;

        let mut replayed = AttemptRecord {
            toolchain: Vec::new(),
            outcome: IN_PROGRESS.to_string(),
            turns: Vec::new(),
            candidate_digest: String::new(),
            promoted: false,
            ..recorded.clone()
        };
        let driven = {
            let run = self.run_in(
                provider,
                &scratch,
                &scratch_rel,
                Some(recorded),
                None,
                recorded.turns.len().max(1),
            );
            run.drive(&mut replayed, first)
        };
        let _ = remove_path(&scratch);
        driven?;

        let differences = divergences(recorded, &replayed);
        if differences.is_empty() {
            return Ok(());
        }
        Err(Error::Invariant(format!(
            "attempt {} does not reproduce from its traces — the recorded evidence was left \
             untouched; {} difference(s): {}",
            recorded.id,
            differences.len(),
            differences.join("; ")
        )))
    }

    /// The candidate of a FINISHED, just-verified attempt — `None` when the
    /// attempt never wrote one. It must still be what the record says: the
    /// CLI promotes from it.
    fn recorded_candidate(
        &self,
        attempt_dir: &Path,
        record: &AttemptRecord,
    ) -> Result<Option<PathBuf>, Error> {
        if record.candidate_digest.is_empty() {
            return Ok(None);
        }
        let candidate = self.stage.candidate_path(attempt_dir);
        let on_disk = match self.stage.candidate_digest(&candidate) {
            Ok(digest) => digest,
            Err(e) if e.is_not_found() => String::new(),
            Err(e) => return Err(e),
        };
        if on_disk != record.candidate_digest {
            return Err(Error::Invariant(format!(
                "attempt {}: {} does not match the record's candidate_digest — the attempt \
                 directory was modified after the attempt finished",
                record.id,
                candidate.display()
            )));
        }
        Ok(Some(candidate))
    }

    /// The recorded attempt a `replay` run verifies: the one pinned by
    /// [`MigrateParams::attempt`], else — among the unit's records whose
    /// FIRST turn has the request key computed from the tree — a finished
    /// one before an `in-progress` one, then an exact model match, then the
    /// lowest sample of the lowest id.
    fn find_recorded(&self, first_key: &str) -> Result<AttemptRecord, Error> {
        let texts = self.stage.texts();
        let first_kind = texts.first_kind;
        let unit_id = self.unit.id.as_str();
        let attempts_dir = self.attempts_dir();
        let matches_key = |record: &AttemptRecord| {
            record
                .turns
                .first()
                .is_some_and(|turn| turn.request_key == first_key)
        };
        let explain = format!(
            "replay verifies RECORDED attempts: it re-runs one from its traces and compares; it \
             never starts a new attempt. The {first_kind} prompt is a function of {}, the model \
             string and max_tokens — all must be what they were when the attempt was recorded",
            texts.prompt_inputs
        );

        if let Some(pinned) = self.params.attempt {
            if !is_clean_segment(pinned) {
                return Err(Error::Invariant(format!(
                    "--attempt {:?} is not an attempt id",
                    printable(pinned, 64)
                )));
            }
            let record = load_record(&attempts_dir.join(pinned), pinned)?
                .filter(|record| record.unit == unit_id)
                .ok_or_else(|| {
                    Error::Invariant(format!(
                        "unit `{unit_id}` has no recorded attempt `{pinned}` under {} — {explain}",
                        attempts_dir.display()
                    ))
                })?;
            if !matches_key(&record) {
                return Err(Error::Invariant(format!(
                    "attempt {pinned} was recorded for a different {first_kind} prompt (request \
                     key {}, now {first_key}) — {explain}",
                    record.turns.first().map_or_else(
                        || "none".to_string(),
                        |turn| printable(&turn.request_key, 16)
                    ),
                )));
            }
            return Ok(record);
        }

        let entries = match std::fs::read_dir(&attempts_dir) {
            Ok(entries) => Some(entries),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::io(&attempts_dir, e)),
        };
        let mut candidates: Vec<AttemptRecord> = Vec::new();
        for entry in entries.into_iter().flatten() {
            let entry = entry.map_err(|e| Error::io(&attempts_dir, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_clean_segment(&name) {
                continue;
            }
            if let Some(record) = load_record(&entry.path(), &name)? {
                if record.unit == unit_id && matches_key(&record) {
                    candidates.push(record);
                }
            }
        }
        // The request key does not cover the provider, so attempts of several
        // providers may match. Finished ones first (only they can be verified),
        // then an exact model match, then base ids in order, each base before
        // its retry samples, those by number.
        let order = |record: &AttemptRecord| {
            let (base, number) = record
                .id
                .rsplit_once(".r")
                .and_then(|(base, _)| {
                    sample_number(&record.id, base).map(|n| (base.to_string(), n))
                })
                .unwrap_or_else(|| (record.id.clone(), 1));
            (
                record.outcome == IN_PROGRESS,
                record.model != self.params.model,
                base,
                number,
            )
        };
        candidates.sort_by_key(order);
        candidates.into_iter().next().ok_or_else(|| {
            Error::Invariant(format!(
                "unit `{unit_id}` has no recorded attempt whose {first_kind} request key is \
                 {first_key} (searched {}) — {explain}",
                attempts_dir.display()
            ))
        })
    }
}

/// Every way `replayed` differs from `recorded` in what a verification
/// compares: per turn `request_key`, `response_hash` and `result`, then
/// `candidate_digest` and `outcome`. Token counts, provider names and the
/// toolchain are NOT compared (a replay reports no usage and may run
/// elsewhere). Recorded values are on-disk data: echoed printable, bounded.
fn divergences(recorded: &AttemptRecord, replayed: &AttemptRecord) -> Vec<String> {
    let show = |value: &str| printable(value, 80);
    let mut out = Vec::new();
    if recorded.turns.len() != replayed.turns.len() {
        out.push(format!(
            "turn count: recorded {}, replayed {}",
            recorded.turns.len(),
            replayed.turns.len()
        ));
    }
    for (index, (was, now)) in recorded.turns.iter().zip(&replayed.turns).enumerate() {
        for (field, was, now) in [
            ("request_key", &was.request_key, &now.request_key),
            ("response_hash", &was.response_hash, &now.response_hash),
            ("result", &was.result, &now.result),
        ] {
            if was != now {
                out.push(format!(
                    "turn {} {field}: recorded {}, replayed {}",
                    index + 1,
                    show(was),
                    show(now)
                ));
            }
        }
    }
    for (field, was, now) in [
        (
            "candidate_digest",
            &recorded.candidate_digest,
            &replayed.candidate_digest,
        ),
        ("outcome", &recorded.outcome, &replayed.outcome),
    ] {
        if was != now {
            out.push(format!(
                "{field}: recorded {}, replayed {}",
                show(was),
                show(now)
            ));
        }
    }
    out
}

/// The record in `dir`, `None` when there is no `attempt.json` yet. The
/// attempts ledger is target-owned input: a record filed under another id
/// than its directory's name is refused rather than trusted.
fn load_record(dir: &Path, id: &str) -> Result<Option<AttemptRecord>, Error> {
    match AttemptRecord::load(dir) {
        Ok(record) if record.id == id => Ok(Some(record)),
        Ok(record) => Err(Error::Invariant(format!(
            "{} holds a record with id {:?}, not `{id}` — the attempts ledger is inconsistent; \
             refusing to touch it",
            dir.join("attempt.json").display(),
            printable(&record.id, 64)
        ))),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e),
    }
}

/// The sample directories that exist for `base` under `attempts_dir`, as
/// (sample number, id) sorted by number: `<base>` is sample 1, `<base>.r<N>`
/// sample N.
fn sample_ids(attempts_dir: &Path, base: &str) -> Result<Vec<(u32, String)>, Error> {
    let entries = match std::fs::read_dir(attempts_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(attempts_dir, e)),
    };
    let mut samples = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(attempts_dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.path().is_dir() {
            continue;
        }
        if let Some(number) = sample_number(&name, base) {
            samples.push((number, name));
        }
    }
    samples.sort();
    Ok(samples)
}

/// `Some(1)` for `base` itself, `Some(N)` for `<base>.r<N>` with N ≥ 2 in
/// canonical decimal, else `None`.
pub(crate) fn sample_number(id: &str, base: &str) -> Option<u32> {
    if id == base {
        return Some(1);
    }
    let digits = id.strip_prefix(base)?.strip_prefix(".r")?;
    let number: u32 = digits.parse().ok()?;
    (number >= 2 && number.to_string() == digits).then_some(number)
}

/// The id a LIVE run records under — decided before anything is created,
/// written, or sent (module docs, "Re-runs never destroy evidence"):
///
/// - no record yet, or an `in-progress` one (an interrupted run): `base`
///   itself — the attempt is started (over) in its directory;
/// - `base` finished and no `retry`: refused, touching nothing;
/// - `base` finished and `retry`: a new sample `<base>.r<N>`, N = 1 + the
///   number of sample dirs that exist (bumped past any directory already
///   there, so a gap left by a hand-deleted sample can never make two
///   samples share a directory). An interrupted retry — the highest sample
///   being `in-progress` — is started over instead of being abandoned next
///   to yet another sample.
fn live_sample_id(attempts_dir: &Path, base: &str, retry: bool) -> Result<String, Error> {
    let finished = |id: &str| -> Result<Option<String>, Error> {
        Ok(load_record(&attempts_dir.join(id), id)?
            .filter(|record| record.outcome != IN_PROGRESS)
            .map(|record| record.outcome))
    };
    let Some(outcome) = finished(base)? else {
        return Ok(base.to_string());
    };
    if !retry {
        return Err(Error::Invariant(format!(
            "attempt {base} already finished ({}); pass --retry to record a new sample",
            printable(&outcome, 32)
        )));
    }
    let samples = sample_ids(attempts_dir, base)?;
    if let Some((number, latest)) = samples.last() {
        if *number >= 2 && finished(latest)?.is_none() {
            return Ok(latest.clone());
        }
    }
    let mut number = u32::try_from(samples.len())
        .unwrap_or(u32::MAX)
        .saturating_add(1)
        .max(2);
    loop {
        let id = format!("{base}.r{number}");
        if std::fs::symlink_metadata(attempts_dir.join(&id)).is_err() {
            return Ok(id);
        }
        number = number.checked_add(1).ok_or_else(|| {
            Error::Invariant(format!("attempt {base}: sample numbers are exhausted"))
        })?;
    }
}

/// Drop what an interrupted run left in the attempt dir it is about to be
/// started over in: `candidate/` and the judge's record (either stage's),
/// which the trajectory regenerates. The ONLY place the executor deletes
/// ledger evidence — and it refuses when the record there is finished,
/// whatever the caller believed.
pub(crate) fn reset_unfinished(work_dir: &Path, id: &str) -> Result<(), Error> {
    if let Some(record) = load_record(work_dir, id)? {
        if record.outcome != IN_PROGRESS {
            return Err(Error::Invariant(format!(
                "internal: attempt {id} is finished ({}); its evidence is never reset",
                printable(&record.outcome, 32)
            )));
        }
    }
    remove_path(&work_dir.join("candidate"))?;
    for judged in JUDGE_RECORDS {
        remove_path(&work_dir.join(judged))?;
    }
    Ok(())
}

/// Outcome when the turn budget is spent without a terminal turn.
fn exhausted_outcome(turns: &[Turn]) -> &'static str {
    if turns.iter().all(|t| t.result == "format") {
        "format"
    } else {
        "red"
    }
}

/// What the next repair turn must be told.
#[derive(Default)]
struct RepairState {
    /// Files of the most recent parseable reply, in spec order.
    files: Option<Vec<String>>,
    /// The failure those files produced.
    failure: Option<Failure>,
    /// Set while the MOST RECENT reply was a format failure.
    format: Option<String>,
}

/// One run's fixed context.
struct Run<'a> {
    job: &'a Job<'a>,
    /// Where completions come from: `params.provider`, or — verifying a
    /// recorded live sample — a replay adapter over that sample's traces.
    provider: &'a ResolvedProvider,
    /// Attempt dir — or the verification scratch dir.
    work_dir: &'a Path,
    /// `work_dir` relative to the unit dir, `/`-joined.
    work_rel: String,
    /// The recorded attempt being verified. `Some` = NOTHING is journaled.
    verifying: Option<&'a AttemptRecord>,
    /// Where each call is recorded as a replayable trace: a live sample's
    /// own trace dir. `None` for trace-backed providers.
    record_traces: Option<PathBuf>,
    /// Turn budget (first turn + repairs).
    max_turns: usize,
    /// Absolute paths replaced in quoted tool output ([`scrub_list`]).
    scrub: Vec<(String, String)>,
}

impl Run<'_> {
    /// Drive the trajectory to its end, journaling after every turn.
    /// Returns whether a candidate was written.
    fn drive(&self, record: &mut AttemptRecord, first: CompletionRequest) -> Result<bool, Error> {
        let provider = self.provider;
        let stage = self.job.stage;
        let texts = stage.texts();
        let mut state = RepairState::default();
        let mut wrote_candidate = false;
        let mut request = first;
        let mut index = 0usize;
        loop {
            if index > 0 {
                request = self.repair_request(&state, &record.turns);
                if let Err(e) = preflight(provider, &request) {
                    if self.verifying.is_some() {
                        return Err(e); // nothing to close: nothing is journaled
                    }
                    // Not a model outcome — but under this profile the
                    // trajectory cannot continue, so it is over, and the
                    // ledger must not claim it is still in progress.
                    let outcome = exhausted_outcome(&record.turns);
                    self.finish(record, outcome)?;
                    return Err(Error::Invariant(format!(
                        "{e} (repair turn {index}; attempt {} closed as `{outcome}`)",
                        record.id
                    )));
                }
            }
            let request_key = TraceAdapter::request_key(&request)?;
            // Verifying: a request that is not the recorded one IS the
            // finding. It is never sent — no adapter could answer it from the
            // record's traces, and the `external` one would file it as a new
            // pending request.
            let recorded_key = self
                .verifying
                .and_then(|recorded| Some((recorded, recorded.turns.get(index)?)))
                .filter(|(_, turn)| turn.request_key != request_key);
            if let Some((recorded, turn)) = recorded_key {
                return Err(Error::Invariant(format!(
                    "attempt {} does not reproduce from its traces — the recorded evidence \
                     was left untouched; turn {} request_key: recorded {}, replayed \
                     {request_key} (the prompt is no longer the recorded one: the judge's \
                     evidence, the toolchain or the harness changed)",
                    recorded.id,
                    index + 1,
                    printable(&turn.request_key, 16)
                )));
            }
            let response = match checked_complete(provider, &request) {
                Ok(response) => response,
                Err(e) => return Err(self.call_failed(record, index, e)),
            };
            // Only a call that passed the context guards is ever recorded:
            // a truncated one must not leave a normal replayable trace.
            if let Some(dir) = &self.record_traces {
                TraceAdapter::record(&prepare_trace_dir(dir)?, &request, &response)?;
            }

            let reported = (response.output_tokens > 0).then_some(response.output_tokens);
            let parsed = emission::parse_spec(
                &response.text,
                response.stop(),
                reported,
                request.max_tokens,
                texts.spec,
            );
            let mut outcome: Option<&'static str> = None;
            let result: &'static str = match parsed {
                ParsedEmission::Blocked(_) => {
                    outcome = Some("blocked");
                    "blocked"
                }
                ParsedEmission::Truncated(_) => {
                    outcome = Some("truncated");
                    "truncated"
                }
                ParsedEmission::Format(message) => {
                    state.format = Some(message);
                    "format"
                }
                ParsedEmission::Files { files, guesses } => {
                    state.format = None;
                    let ctx = RunCtx {
                        work_dir: self.work_dir,
                        work_rel: self.work_rel.clone(),
                        verifying: self.verifying.is_some(),
                        scrub: &self.scrub,
                    };
                    let judged = stage.judge(&ctx, &files, record)?;
                    wrote_candidate |= judged.wrote_candidate;
                    match judged.failure {
                        None => {
                            outcome = Some("green");
                            "green"
                        }
                        Some(mut failure) => {
                            // What the parser had to guess is feedback too:
                            // the next reply should not need the leniency.
                            failure.evidence.push_str(&emission_notes(&guesses));
                            let class = failure.class;
                            state.files = Some(files);
                            state.failure = Some(failure);
                            class
                        }
                    }
                }
            };

            let usage = |n: u64| (provider.live && n > 0).then_some(n);
            record.turns.push(Turn {
                kind: if index == 0 {
                    texts.first_kind
                } else {
                    "repair"
                }
                .to_string(),
                result: result.to_string(),
                request_key,
                response_hash: hash::bytes_hash(response.text.as_bytes()),
                input_tokens: usage(response.input_tokens),
                output_tokens: usage(response.output_tokens),
            });
            if outcome.is_none() && index + 1 >= self.max_turns {
                outcome = Some(exhausted_outcome(&record.turns));
            }
            if let Some(outcome) = outcome {
                self.finish(record, outcome)?;
                return Ok(wrote_candidate);
            }
            self.store(record)?;
            index += 1;
        }
    }

    /// The error to return when the call of turn `index` failed.
    ///
    /// - Verifying: propagated unchanged — nothing was journaled, so there
    ///   is nothing to close.
    /// - "prompt truncated by server" (or a preflight refusal): the turn is
    ///   void, and the attempt is left RESUMABLE rather than closed — this
    ///   is a fault of the endpoint's configuration, not an outcome of the
    ///   trajectory. When no turn completed, the attempt dir this run just
    ///   (re)created holds nothing but an empty `in-progress` record, and
    ///   is removed again; otherwise the record — journaled after the last
    ///   completed turn — already says `in-progress` with the turns so far,
    ///   and stays. The error is propagated with that fact appended.
    /// - Every other adapter error (the `external` hand-off's "awaiting
    ///   response" above all) is propagated unchanged; the record says
    ///   `in-progress`, which is exactly right.
    fn call_failed(&self, record: &AttemptRecord, index: usize, e: Error) -> Error {
        if self.verifying.is_some() || !is_context_error(&e) {
            return e;
        }
        if !record.turns.is_empty() {
            return Error::Invariant(format!(
                "{e} (repair turn {index}; attempt {} stays `in-progress` with the {} turn(s) \
                 completed — fix the endpoint and re-run)",
                record.id,
                record.turns.len()
            ));
        }
        let removed = remove_path(self.work_dir).is_ok();
        if let Some(attempts_dir) = self.work_dir.parent() {
            // Only ever removes an EMPTY attempts dir (this run's own).
            let _ = std::fs::remove_dir(attempts_dir);
        }
        Error::Invariant(format!(
            "{e} ({} turn; no turn completed, so the empty attempt record {} {})",
            self.job.stage.texts().first_kind,
            record.id,
            if removed {
                "was removed again"
            } else {
                "could not be removed"
            }
        ))
    }

    /// Journal the record (a no-op while verifying).
    fn store(&self, record: &AttemptRecord) -> Result<(), Error> {
        if self.verifying.is_none() {
            record.store(self.work_dir)?;
        }
        Ok(())
    }

    /// Close the record. `promoted` is the CLI's to set, after a promotion;
    /// an attempt that ends here was never finished before, so never
    /// promoted.
    fn finish(&self, record: &mut AttemptRecord, outcome: &str) -> Result<(), Error> {
        record.outcome = outcome.to_string();
        record.promoted = false;
        self.store(record)
    }

    /// The stateless repair request: the pinned sections again, then the
    /// current files, the failure, and the turn history.
    fn repair_request(&self, state: &RepairState, turns: &[Turn]) -> CompletionRequest {
        let texts = self.job.stage.texts();
        let current = match &state.files {
            Some(files) => emission::render_spec(texts.spec, files),
            None => texts.no_current.to_string(),
        };
        let format_failure = FORMAT_EXPLANATION;
        let (class, explanation, evidence) = match (&state.format, &state.failure) {
            (Some(message), None) => ("format", format_failure, format!("{message}\n")),
            (Some(message), Some(earlier)) => (
                "format",
                format_failure,
                format!(
                    "{message}\n{} failed with class {} ({}):\n{}",
                    texts.earlier, earlier.class, earlier.explanation, earlier.evidence
                ),
            ),
            (None, Some(failure)) => (failure.class, failure.explanation, failure.evidence.clone()),
            (None, None) => (
                "format",
                format_failure,
                "no usable reply so far\n".to_string(),
            ),
        };
        let history: Vec<String> = turns
            .iter()
            .enumerate()
            .map(|(i, turn)| format!("{}. {} -> {}\n", i + 1, turn.kind, turn.result))
            .collect();
        let user = format!(
            "{}\n[{}]\n{current}\n[FAILURE CLASS]\n{class} — {explanation}\n\n\
             [EVIDENCE]\n{evidence}\n[HISTORY]\n{}\n[TASK]\n{}\n",
            self.job.pinned,
            texts.current_section,
            history.concat(),
            texts.repair_task
        );
        self.job.request(user)
    }
}

/// `[FAILURE CLASS]` explanation of a format failure (every stage).
pub(crate) const FORMAT_EXPLANATION: &str =
    "your previous reply did not follow the emission contract, so nothing was built";

/// blake3(system ‖ NUL ‖ user), rendered `blake3:<hex>`.
pub(crate) fn prompt_digest(request: &CompletionRequest) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(request.system.as_bytes());
    hasher.update(b"\0");
    hasher.update(request.user.as_bytes());
    format!("{}{}", hash::HASH_PREFIX, hasher.finalize().to_hex())
}

/// One source file of the unit's include closure.
pub(crate) struct SourceFile {
    /// Repo-relative path, as listed by the facts.
    pub(crate) path: String,
    /// Exact file bytes (hashed, and sent lossily decoded).
    pub(crate) bytes: Vec<u8>,
}

/// Read the unit's include closure. `facts.jsonl` and `plan.toml` are
/// target-owned, and whatever is read here is SENT TO THE PROVIDER — so a
/// path must be clean and relative, and must still be inside the target
/// root AND inside `[target] source_dir` once symlinks are resolved
/// (docs/M4-DESIGN.md R2: prompt-bound reads are confined to `source_dir`,
/// so a hostile include cannot pull held-out material into a prompt).
/// Anything else in the closure is a harness error ([`Error::InvalidPlan`]).
pub(crate) fn read_sources(
    root: &Path,
    source_dir: &str,
    facts: &Facts,
    unit: &Unit,
) -> Result<Vec<SourceFile>, Error> {
    let closure = facts.include_closure(&unit.files);
    if closure.is_empty() {
        return Err(Error::InvalidPlan(format!(
            "unit `{}` has no source files",
            unit.id
        )));
    }
    let joined = root.join(source_dir);
    let confined = joined.canonicalize().map_err(|e| Error::io(&joined, e))?;
    let mut sources = Vec::with_capacity(closure.len());
    for path in closure {
        if !is_clean_relative_path(&path) {
            return Err(Error::Invariant(format!(
                "unit `{}`: source path {path:?} is not a clean relative path — refusing to \
                 read it (re-run `harness scan`)",
                unit.id
            )));
        }
        let joined = root.join(&path);
        let resolved = joined.canonicalize().map_err(|e| Error::io(&joined, e))?;
        if !resolved.starts_with(root) {
            return Err(Error::InvalidPlan(format!(
                "unit `{}`: source path {path:?} resolves to {}, outside the target root — \
                 refusing to send it to a model provider",
                unit.id,
                resolved.display()
            )));
        }
        if !resolved.starts_with(&confined) {
            return Err(Error::InvalidPlan(format!(
                "unit `{}`: source path {path:?} is outside the target's source_dir {:?} — \
                 prompt-bound reads are confined to it; refusing to send it to a model provider",
                unit.id,
                printable(source_dir, 256)
            )));
        }
        let bytes = std::fs::read(&resolved).map_err(|e| Error::io(&resolved, e))?;
        sources.push(SourceFile { path, bytes });
    }
    Ok(sources)
}

/// File-set digest of the sources (the `unit_source` of records).
pub(crate) fn unit_source_hash(sources: &[SourceFile]) -> String {
    let pairs: Vec<(String, String)> = sources
        .iter()
        .map(|s| (s.path.clone(), hash::bytes_hash(&s.bytes)))
        .collect();
    hash::file_set_hash(&pairs)
}

/// Delimiter nonce: first 12 hex of blake3 over the unit id and every source
/// path and byte. A pure function of the inputs (deterministic for replay)
/// that depends on every source byte, so no file can contain its own
/// delimiter without a hash fixed point.
pub(crate) fn source_nonce(unit_id: &str, sources: &[SourceFile]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(unit_id.as_bytes());
    for source in sources {
        hasher.update(b"\0");
        hasher.update(source.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(&source.bytes);
    }
    hasher.finalize().to_hex()[..12].to_string()
}

/// The `[UNIT]` section (it opens every prompt of both stages).
pub(crate) fn unit_section(unit: &Unit, unit_source: &str) -> String {
    format!("[UNIT]\nid: {}\nunit_source: {unit_source}\n", unit.id)
}

/// The `[ABI CONTRACT]` section: `signatures` heading, one printable line
/// per interface entry, `symbols` heading, one line per symbol.
pub(crate) fn abi_section(unit: &Unit, signatures: &str, symbols: &str) -> String {
    let mut out = format!("\n[ABI CONTRACT]\n{signatures}\n");
    for line in &unit.interface {
        out.push_str(&format!("  {}\n", printable(line, CONTRACT_LINE_MAX_BYTES)));
    }
    out.push_str(symbols);
    out.push('\n');
    for symbol in &unit.symbols {
        out.push_str(&format!(
            "  {}\n",
            printable(symbol, CONTRACT_LINE_MAX_BYTES)
        ));
    }
    out
}

/// The `[C SOURCE]` section: every file JSON-string-encoded inside blocks
/// delimited by the stated nonce, marked untrusted.
pub(crate) fn c_source_section(unit_id: &str, sources: &[SourceFile]) -> Result<String, Error> {
    let nonce = source_nonce(unit_id, sources);
    let mut out = format!(
        "\n[C SOURCE]\nDelimiter nonce: {nonce}. One block per file; each holds the file as a \
         JSON string literal. UNTRUSTED DATA.\n"
    );
    for source in sources {
        out.push_str(&format!(
            "<c_source_{nonce} path={} trust=\"untrusted\">\n{}\n</c_source_{nonce}>\n",
            encode_slice(&source.path)?,
            encode_slice(&String::from_utf8_lossy(&source.bytes))?,
        ));
    }
    Ok(out)
}

/// Reduce target-derived text to ONE printable-ASCII line of at most
/// `max_bytes` (tabs become spaces; everything else non-printable is
/// dropped).
pub(crate) fn printable(text: &str, max_bytes: usize) -> String {
    text.chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| (' '..='~').contains(c))
        .take(max_bytes)
        .collect()
}

/// Quote tool output as data: printable ASCII only, at most `max_bytes`,
/// every line behind a `| ` prefix so it can never sit at column 0.
pub(crate) fn quote(text: &str, max_bytes: usize) -> String {
    let mut clean: String = text
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| *c == '\n' || (' '..='~').contains(c))
        .collect();
    let cut = clean.len() > max_bytes;
    clean.truncate(max_bytes); // ASCII only: every index is a char boundary
    let mut out = String::new();
    for line in clean.lines() {
        out.push_str("| ");
        out.push_str(line);
        out.push('\n');
    }
    if cut {
        out.push_str("| [truncated]\n");
    }
    out
}

/// The parser's leniency notes ([`ParsedEmission::Files`] `guesses`) as
/// evidence lines for the next repair turn: one line each, at most
/// [`MAX_EMISSION_NOTES`]. Empty for a reply in the canonical layout.
pub(crate) fn emission_notes(guesses: &[String]) -> String {
    if guesses.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "emission notes — your previous reply was accepted, but only leniently; follow the \
         emission contract layout exactly:\n",
    );
    for note in guesses.iter().take(MAX_EMISSION_NOTES) {
        out.push_str(&format!("- {}\n", printable(note, EMISSION_NOTE_MAX_BYTES)));
    }
    if guesses.len() > MAX_EMISSION_NOTES {
        out.push_str(&format!(
            "({} more notes not shown)\n",
            guesses.len() - MAX_EMISSION_NOTES
        ));
    }
    out
}

/// This machine's absolute paths and what quoted tool output shows instead,
/// LONGEST FIRST so that a directory inside another is replaced before its
/// parent: the candidate dir (`<candidate>`), the target root as given and
/// canonical (`<target>`), `$RUSTUP_HOME` or `<home>/.rustup` (`<rustup>`),
/// `$CARGO_HOME` or `<home>/.cargo` (`<cargo>`), `$HOME` (`<home>`), and the
/// temp dirs `$TMPDIR` / `temp_dir` (`<tmp>`) — each also in its canonical
/// form (`/var/…` is `/private/var/…` on macOS). They would make prompts,
/// and so trace keys, depend on who runs the harness where, and they are
/// nobody's business. The judges scrub their details too; this list is the
/// executor's own defense in depth. Relative and root (`/`) values are
/// ignored: replacing them could only mangle text.
pub(crate) fn scrub_list(
    candidate_dir: &Path,
    root: &Path,
    target_root: &Path,
    env: EnvLookup<'_>,
    temp_dir: &Path,
) -> Vec<(String, String)> {
    let var = |name: &str| {
        env(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let home = var("HOME").or_else(|| var("USERPROFILE"));
    let under_home = |dir: &str| home.as_ref().map(|home| home.join(dir));
    let host: [(Option<PathBuf>, &'static str); 5] = [
        (
            var("RUSTUP_HOME").or_else(|| under_home(".rustup")),
            "<rustup>",
        ),
        (
            var("CARGO_HOME").or_else(|| under_home(".cargo")),
            "<cargo>",
        ),
        (home.clone(), "<home>"),
        (var("TMPDIR"), "<tmp>"),
        (Some(temp_dir.to_path_buf()), "<tmp>"),
    ];
    let mut paths: Vec<(PathBuf, &'static str)> = vec![
        (candidate_dir.to_path_buf(), "<candidate>"),
        (root.to_path_buf(), "<target>"),
        (target_root.to_path_buf(), "<target>"),
    ];
    paths.extend(
        host.into_iter()
            .filter_map(|(path, label)| path.map(|path| (path, label))),
    );

    let mut list: Vec<(String, String)> = Vec::new();
    for (path, label) in paths {
        if !path.is_absolute() {
            continue;
        }
        for form in [Some(path.clone()), path.canonicalize().ok()]
            .into_iter()
            .flatten()
        {
            let text = form.display().to_string();
            let text = text.trim_end_matches('/');
            if text.len() > 1 && !list.iter().any(|(known, _)| known == text) {
                list.push((text.to_string(), label.to_string()));
            }
        }
    }
    // Stable: equally long paths keep the precedence of the order above.
    list.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
    list
}

/// Replace every path of `scrub` (see [`scrub_list`]) in `text`.
pub(crate) fn scrub_paths(scrub: &[(String, String)], text: &str) -> String {
    let mut text = text.to_string();
    for (path, label) in scrub {
        text = text.replace(path.as_str(), label.as_str());
    }
    text
}

/// Remove whatever is at `path` WITHOUT following a symlink; a missing path
/// is fine.
pub(crate) fn remove_path(path: &Path) -> Result<(), Error> {
    let removed = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    };
    removed.map_err(|e| Error::io(path, e))
}

/// Create a FRESH `<work_dir>/candidate/` directory: whatever is at that
/// path (a previous candidate, a file, a symlink) is removed first.
pub(crate) fn fresh_candidate_dir(work_dir: &Path) -> Result<PathBuf, Error> {
    let dir = work_dir.join("candidate");
    remove_path(&dir)?;
    std::fs::create_dir(&dir).map_err(|e| Error::io(&dir, e))?;
    Ok(dir)
}

/// Write `content` to a NEW file at `path` (`create_new`: nothing is ever
/// written through a pre-existing path).
pub(crate) fn write_new(path: &Path, content: &str) -> Result<(), Error> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Error::io(path, e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| Error::io(path, e))
}

/// Create a live sample's trace dir (`<traces_dir>/<attempt-id>`, made only
/// once there is a call to record) and check that it IS that directory: the
/// attempt id is content-derived, hence predictable, and a hostile target
/// could have committed a symlink under that name to redirect the writes.
fn prepare_trace_dir(dir: &Path) -> Result<PathBuf, Error> {
    let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
        return Err(Error::Invariant(format!(
            "trace dir {} has no parent",
            dir.display()
        )));
    };
    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    let expected = parent
        .canonicalize()
        .map_err(|e| Error::io(parent, e))?
        .join(name);
    match std::fs::create_dir(&expected) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(Error::io(&expected, e)),
    }
    let resolved = expected
        .canonicalize()
        .map_err(|e| Error::io(&expected, e))?;
    if resolved != expected {
        return Err(Error::InvalidPlan(format!(
            "{} resolves to {} — refusing to record traces through a symlink",
            expected.display(),
            resolved.display()
        )));
    }
    Ok(expected)
}

/// Create `migration/units/<unit>/<rel…>` one level at a time, checking
/// after each level that it IS that directory once symlinks are resolved
/// (docs/SCHEMAS.md "Trust boundaries": every write destination is checked
/// to be inside the unit dir after canonicalization). Because each parent is
/// verified before its child is created, nothing is ever created through a
/// symlink a hostile target committed. `ledger` must be rooted at the
/// canonical target root.
pub(crate) fn prepare_dir(
    ledger: &Ledger,
    unit_id: &str,
    rel: &[String],
) -> Result<PathBuf, Error> {
    let unit_dir = ledger.unit_dir(unit_id);
    let mut chain: Vec<PathBuf> = vec![ledger.dir()];
    if let Some(units) = unit_dir.parent().filter(|p| *p != ledger.dir()) {
        chain.push(units.to_path_buf());
    }
    chain.push(unit_dir.clone());
    let mut dir = unit_dir;
    for segment in rel {
        dir.push(segment);
        chain.push(dir.clone());
    }
    for level in &chain {
        match std::fs::create_dir(level) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::io(level, e)),
        }
        let resolved = level.canonicalize().map_err(|e| Error::io(level, e))?;
        if resolved != *level {
            return Err(Error::InvalidPlan(format!(
                "unit `{unit_id}`: {} resolves to {} — refusing to write model output through \
                 a symlink out of the unit directory",
                level.display(),
                resolved.display()
            )));
        }
    }
    Ok(dir)
}
