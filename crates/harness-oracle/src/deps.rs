//! The dependency inputs of a unit's builds (docs/BEYOND-LEAVES-DESIGN.md
//! §1, §2): the pieces it calls, transitively, in the plan's execution
//! order, each linked **as the program stands** — its promoted Rust
//! staticlib when its status is verified or merged and its crate builds,
//! else its C files — or, for driver validation, each one's C. Every build
//! of the judge takes them from here, so none can forget a callee.
//!
//! Two halves: [`plan`] reads the plan and decides each piece's form
//! without running anything (so the sandbox profile can grant the
//! callees' `target/` folders before cargo runs); [`inputs`] builds the
//! accepted crates the way verify builds a unit's own and falls back to a
//! callee's C when its crate does not build. Which piece calls which comes
//! from harness-core ([`harness_core::deps`]), so `state status` and the
//! cockpit compare exactly what a verdict recorded.

use crate::exec::Runner;
use harness_core::config::TargetContext;
use harness_core::deps::{self as core_deps, Dependency, DependencyKind, Stand};
use harness_core::error::Error;
use harness_core::facts::Facts;
use harness_core::ledger::Ledger;
use harness_core::Unit;
use std::path::{Path, PathBuf};

/// One callee as [`plan`] decided it, before any build.
#[derive(Debug, Clone)]
pub(crate) struct Planned {
    /// The callee's unit id.
    pub unit: String,
    /// Its C files, canonical and inside the target root.
    pub c_files: Vec<PathBuf>,
    /// Its `c` digest.
    pub c_digest: String,
    /// Its accepted crate (canonical folder, `target/` folder), when the
    /// stand links Rust and it has one.
    pub rust: Option<(PathBuf, PathBuf)>,
    /// Its plan symbols: what the unit's Rust may import from it.
    pub symbols: Vec<String>,
}

/// A unit's callees, decided ([`plan`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct Plan {
    /// In execution order, leaves first.
    pub planned: Vec<Planned>,
}

impl Plan {
    /// The callees' crate `target/` folders and `Cargo.lock` files: what
    /// the tool sandbox must let cargo write.
    pub(crate) fn writes(&self) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for (crate_dir, target_dir) in self.planned.iter().filter_map(|p| p.rust.as_ref()) {
            dirs.push(target_dir.clone());
            files.push(crate_dir.join("Cargo.lock"));
        }
        (dirs, files)
    }

    /// Every symbol of every callee: what the unit's Rust may import.
    pub(crate) fn symbols(&self) -> Vec<String> {
        self.planned
            .iter()
            .flat_map(|p| p.symbols.iter().cloned())
            .collect()
    }
}

/// One callee as the builds link it.
#[derive(Debug, Clone)]
pub(crate) struct Input {
    /// What the verdict records.
    pub dependency: Dependency,
    /// What the link takes: its C files, or its staticlib.
    pub link: Vec<PathBuf>,
    /// Its C files, which its staticlib replaces in a whole program.
    pub replaced: Vec<PathBuf>,
}

/// Decide `unit`'s callees under `stand` — reading the plan only when the
/// unit depends on something. Every path canonical and inside `root` (the
/// canonical target root); no subprocess runs.
pub(crate) fn plan(
    target: &TargetContext,
    root: &Path,
    facts: &Facts,
    unit: &Unit,
    stand: Stand,
) -> Result<Plan, Error> {
    let ledger = Ledger::of_under(target, root.to_path_buf());
    let Some(plan) = core_deps::plan_for(&ledger, unit)? else {
        return Ok(Plan::default());
    };
    let mut planned = Vec::new();
    for dep in core_deps::transitive(&plan, unit)? {
        let mut c_files = Vec::new();
        for rel in core_deps::c_files(dep) {
            c_files.push(crate::inside(
                &unit.id,
                &format!("file of the piece it calls ({})", dep.id),
                &root.join(&rel),
                root,
            )?);
        }
        let rust = match (stand, core_deps::accepted_crate(&ledger, dep)) {
            (Stand::AsItStands, Some(raw)) => {
                let unit_dir = ledger.unit_dir(&dep.id);
                let unit_dir = unit_dir
                    .canonicalize()
                    .map_err(|e| Error::io(&unit_dir, e))?;
                let crate_dir = crate::inside(
                    &unit.id,
                    &format!("crate of the piece it calls ({})", dep.id),
                    &raw,
                    &unit_dir,
                )?;
                let target_dir = crate::prepare_target_dir(&crate_dir)?;
                Some((crate_dir, target_dir))
            }
            _ => None,
        };
        planned.push(Planned {
            unit: dep.id.clone(),
            c_files,
            c_digest: core_deps::c_digest(target, facts, dep),
            rust,
            symbols: dep.symbols.clone(),
        });
    }
    Ok(Plan { planned })
}

/// Every OTHER accepted piece of the program (verified or merged, its crate
/// present), in execution order: what the whole program links as its Rust,
/// as the program stands (docs/BEYOND-LEAVES-DESIGN.md §2, last row) — the
/// unit itself is the candidate. Empty when the plan cannot be read (a
/// target verified without one): the program is then all C but the unit.
pub(crate) fn accepted(
    target: &TargetContext,
    root: &Path,
    facts: &Facts,
    unit: &Unit,
) -> Result<Plan, Error> {
    let ledger = Ledger::of_under(target, root.to_path_buf());
    let Ok(plan) = harness_core::Plan::load(&ledger.plan_path()) else {
        return Ok(Plan::default());
    };
    let Ok(order) = plan.execution_order() else {
        return Ok(Plan::default());
    };
    let mut planned = Vec::new();
    for other in order.into_iter().filter(|u| u.id != unit.id) {
        let Some(raw) = core_deps::accepted_crate(&ledger, other) else {
            continue;
        };
        let unit_dir = ledger.unit_dir(&other.id);
        let unit_dir = unit_dir
            .canonicalize()
            .map_err(|e| Error::io(&unit_dir, e))?;
        let crate_dir = crate::inside(&unit.id, "crate of an accepted piece", &raw, &unit_dir)?;
        let target_dir = crate::prepare_target_dir(&crate_dir)?;
        let mut c_files = Vec::new();
        for rel in core_deps::c_files(other) {
            c_files.push(crate::inside(
                &unit.id,
                &format!("file of an accepted piece ({})", other.id),
                &root.join(&rel),
                root,
            )?);
        }
        planned.push(Planned {
            unit: other.id.clone(),
            c_files,
            c_digest: core_deps::c_digest(target, facts, other),
            rust: Some((crate_dir, target_dir)),
            symbols: other.symbols.clone(),
        });
    }
    Ok(Plan { planned })
}

/// The program as it stands, for the whole-program builds: `c_files` (the
/// program's) without the C of every `others` piece linked as Rust, and
/// those pieces' staticlibs (callers' first) to link after the unit's own.
pub(crate) fn as_it_stands(c_files: &[PathBuf], others: &[Input]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let rust: Vec<&Input> = others
        .iter()
        .filter(|i| i.dependency.kind == DependencyKind::Rust)
        .collect();
    let mut planned_c: Vec<&PathBuf> = Vec::new();
    for i in &rust {
        planned_c.extend(i.replaced.iter());
    }
    let kept = c_files
        .iter()
        .filter(|c| !planned_c.contains(c))
        .cloned()
        .collect();
    let libs = rust
        .iter()
        .rev()
        .flat_map(|i| i.link.iter().cloned())
        .collect();
    (kept, libs)
}

/// Build what `plan` decided: each accepted crate with cargo exactly as
/// verify builds a unit's own (the unit-crate check first, `--release
/// --offline` into its own `target/`, under `runner`'s tool profile, which
/// must grant [`Plan::writes`]); a crate that is refused or does not build
/// is linked as its C. Only an interrupt is an error.
pub(crate) fn inputs(
    target: &TargetContext,
    runner: &Runner,
    plan: &Plan,
) -> Result<Vec<Input>, Error> {
    let mut out = Vec::with_capacity(plan.planned.len());
    for p in &plan.planned {
        let built = match &p.rust {
            None => None,
            Some((crate_dir, target_dir)) => {
                let lib = crate::unit_crate::check_unit_crate(&p.unit, crate_dir).and_then(|()| {
                    crate::build_staticlib(
                        runner,
                        runner.tool_profile.as_deref(),
                        crate_dir,
                        target_dir,
                    )
                });
                match lib {
                    Ok(lib) => Some((lib, core_deps::rust_digest(target, crate_dir))),
                    Err(Error::Interrupted) => return Err(Error::Interrupted),
                    Err(_) => None,
                }
            }
        };
        out.push(match built {
            Some((lib, digest)) => Input {
                dependency: Dependency {
                    unit: p.unit.clone(),
                    kind: DependencyKind::Rust,
                    digest,
                },
                link: vec![lib],
                replaced: p.c_files.clone(),
            },
            None => Input {
                dependency: Dependency {
                    unit: p.unit.clone(),
                    kind: DependencyKind::C,
                    digest: p.c_digest.clone(),
                },
                link: p.c_files.clone(),
                replaced: p.c_files.clone(),
            },
        });
    }
    Ok(out)
}

/// What the builds link, in order: every callee's C files, then every
/// callee's staticlib (callers' before their callees', so a one-pass
/// linker resolves each archive's references from the ones after it; on
/// Linux the staticlibs are grouped anyway, [`crate::perf::build`]).
pub(crate) fn link_paths(inputs: &[Input]) -> Vec<PathBuf> {
    let c = inputs
        .iter()
        .filter(|i| i.dependency.kind == DependencyKind::C)
        .flat_map(|i| i.link.iter().cloned());
    let rust = inputs
        .iter()
        .rev()
        .filter(|i| i.dependency.kind == DependencyKind::Rust)
        .flat_map(|i| i.link.iter().cloned());
    c.chain(rust).collect()
}

/// What a verdict records.
pub(crate) fn records(inputs: &[Input]) -> Vec<Dependency> {
    inputs.iter().map(|i| i.dependency.clone()).collect()
}

/// The words of a failed link for a check's detail: the linker's first two
/// lines after the compiler's own "cc failed (…)" line, joined.
pub(crate) fn linker_words(words: &str) -> String {
    let lines: Vec<&str> = words
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("cc failed"))
        .take(2)
        .collect();
    if lines.is_empty() {
        words.trim().to_string()
    } else {
        lines.join(" / ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(unit: &str, kind: DependencyKind, link: &str) -> Input {
        Input {
            dependency: Dependency {
                unit: unit.into(),
                kind,
                digest: "blake3:x".into(),
            },
            link: vec![PathBuf::from(link)],
            replaced: vec![PathBuf::from(format!("{unit}.c"))],
        }
    }

    #[test]
    fn c_files_come_first_then_staticlibs_callers_first() {
        let inputs = vec![
            input("a", DependencyKind::Rust, "liba.a"),
            input("b", DependencyKind::C, "b.c"),
            input("c", DependencyKind::Rust, "libc_.a"),
        ];
        assert_eq!(
            link_paths(&inputs),
            [
                PathBuf::from("b.c"),
                PathBuf::from("libc_.a"),
                PathBuf::from("liba.a")
            ]
        );
    }

    #[test]
    fn the_program_as_it_stands_swaps_only_the_rust_pieces() {
        let others = vec![
            input("a", DependencyKind::Rust, "liba.a"),
            input("b", DependencyKind::C, "b.c"),
            input("c", DependencyKind::Rust, "libc_.a"),
        ];
        let files: Vec<PathBuf> = ["a.c", "b.c", "c.c", "main.c"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let (kept, libs) = as_it_stands(&files, &others);
        assert_eq!(kept, [PathBuf::from("b.c"), PathBuf::from("main.c")]);
        assert_eq!(libs, [PathBuf::from("libc_.a"), PathBuf::from("liba.a")]);
    }

    #[test]
    fn linker_words_skip_the_compilers_line() {
        let words = "cc failed (exit status: 1):\nUndefined symbols for architecture arm64:\n  \
                     \"_callee\", referenced from:\n      _caller in caller.o\n";
        assert_eq!(
            linker_words(words),
            "Undefined symbols for architecture arm64: / \"_callee\", referenced from:"
        );
    }
}
