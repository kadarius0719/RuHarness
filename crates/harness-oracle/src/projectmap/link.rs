//! The map's link checks (docs/PROJECT-MAP-DESIGN.md §3.5): the real
//! [`Linker`]. Each file of a closure is compiled once per run **exactly as
//! the map compiled it** — the map's own argv ([`super::compile_argv`]):
//! the judge's base flags, the file's own [`FileFacts::flags`], its include
//! folders (`-idirafter` for a folder holding one of the configuration's
//! `system_headers`) — into a fresh folder under the map's sandboxed
//! runner; an object over [`MAX_OBJECT_BYTES`] is refused, and an object is
//! deleted as soon as no closure still to be linked needs it. The objects
//! are linked into one program with the outside libraries guessed from the
//! outside symbols (on Apple with `-Wl,-ignore_auto_link`, so an object's
//! own `.linker_option` cannot choose a library). Nothing built is ever
//! run. Every compile, link and probe stops at the map's deadline.
//!
//! The result comes from the symbol facts and the linker's exit status,
//! never from its text (which can carry project text): when a link fails,
//! `doubled` is every symbol two linked files define strongly,
//! `not_compiled` every file that did not compile for the link, and
//! `missing` every unresolved symbol the system's libraries do not provide,
//! found by linking a trivial program that requires a group of them
//! (`-u` on Mach-O, `--require-defined` elsewhere) and halving a group that
//! fails; a name the probe budget could not decide is `not_checked`, never
//! `missing`.

use super::closure::{self, Analysis, Input, Linked, Linker};
use super::{compile_argv, FileFacts, Fresh, MAX_OBJECT_BYTES};
use crate::exec::{ChildEnd, Runner};
use crate::objsyms;
use harness_core::error::Error;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The wall-clock limit of one compile or link (§3.10).
pub const LINK_TIMEOUT_SECS: u64 = 120;
/// The most probe links one failed link may spend finding its missing
/// symbols; symbols still undecided past it are "not checked", never
/// missing.
pub const MAX_PROBES: usize = 64;

/// libm's names (C99 §7.12), matched with an `f` or `l` suffix too.
const LIBM: &[&str] = &[
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "cbrt",
    "ceil",
    "copysign",
    "cos",
    "cosh",
    "erf",
    "erfc",
    "exp",
    "exp2",
    "expm1",
    "fabs",
    "fdim",
    "floor",
    "fma",
    "fmax",
    "fmin",
    "fmod",
    "frexp",
    "hypot",
    "ilogb",
    "ldexp",
    "lgamma",
    "llrint",
    "llround",
    "log",
    "log10",
    "log1p",
    "log2",
    "logb",
    "lrint",
    "lround",
    "modf",
    "nan",
    "nearbyint",
    "nextafter",
    "nexttoward",
    "pow",
    "remainder",
    "remquo",
    "rint",
    "round",
    "scalbln",
    "scalbn",
    "sin",
    "sinh",
    "sqrt",
    "tan",
    "tanh",
    "tgamma",
    "trunc",
];

fn is_libm(sym: &str) -> bool {
    LIBM.contains(&sym)
        || sym
            .strip_suffix('f')
            .or_else(|| sym.strip_suffix('l'))
            .is_some_and(|base| LIBM.contains(&base))
}

/// The outside libraries guessed from `unresolved` (§3.5 step 1): `-lm`
/// for a libm name, `-lz` for `deflate*`/`inflate*` when `zlib` links on
/// this system, `-lpthread` for `pthread_*` where the platform needs it
/// (not on Apple's, whose libc holds it).
pub fn guess_libs(unresolved: &[String], zlib: bool, apple: bool) -> Vec<&'static str> {
    let mut libs = Vec::new();
    if unresolved.iter().any(|s| is_libm(s)) {
        libs.push("-lm");
    }
    if zlib
        && unresolved
            .iter()
            .any(|s| s.starts_with("deflate") || s.starts_with("inflate"))
    {
        libs.push("-lz");
    }
    if !apple && unresolved.iter().any(|s| s.starts_with("pthread_")) {
        libs.push("-lpthread");
    }
    libs
}

fn path_arg(p: &Path) -> Result<String, Error> {
    crate::path_str(p).map(str::to_string)
}

/// What the real linker reads of the map: the walked paths and the
/// configuration's `system_headers` (for the map's own compile argv), and
/// the deadline.
#[derive(Debug, Clone)]
pub struct LinkSetup<'a> {
    /// Every walked `.c` and `.h`, relative to the root.
    pub walked: BTreeSet<&'a str>,
    /// The configuration's `system_headers`.
    pub system_headers: &'a [String],
    /// When the map's time budget runs out.
    pub deadline: Instant,
}

/// The real linker: `cc` under the map's runner, every object and program
/// in its own fresh folder, removed with it.
pub struct CcLinker<'a> {
    root: PathBuf,
    runner: Runner,
    setup: LinkSetup<'a>,
    fresh: Fresh,
    apple: bool,
    /// Objects by file path (`None`: the compile failed or the object was
    /// too large).
    objects: BTreeMap<String, Option<PathBuf>>,
    /// The trivial program the probes link.
    probe: Option<PathBuf>,
    zlib: Option<bool>,
    links: usize,
    /// Objects compiled so far (each in its own folder `o<n>`).
    made: usize,
    /// The deadline passed: nothing more is compiled or linked.
    ran_out: bool,
}

impl<'a> CcLinker<'a> {
    /// A linker for the project at `root` (canonical).
    pub fn new(root: &Path, setup: LinkSetup<'a>) -> Result<CcLinker<'a>, Error> {
        CcLinker::new_in(root, setup, &std::env::temp_dir())
    }

    /// [`CcLinker::new`] with the fresh folder made under `fresh_parent`.
    pub(crate) fn new_in(
        root: &Path,
        setup: LinkSetup<'a>,
        fresh_parent: &Path,
    ) -> Result<CcLinker<'a>, Error> {
        let root = root.canonicalize().map_err(|e| Error::io(root, e))?;
        let fresh = Fresh::make(fresh_parent)?;
        let runner = Runner::map(
            &root,
            &fresh.0,
            &["cc"],
            Duration::from_secs(LINK_TIMEOUT_SECS),
        )?;
        Ok(CcLinker {
            root,
            runner,
            setup,
            fresh,
            apple: cfg!(target_vendor = "apple"),
            objects: BTreeMap::new(),
            probe: None,
            zlib: None,
            links: 0,
            made: 0,
            ran_out: false,
        })
    }

    /// The deadline passed (and is now recorded as passed).
    fn out_of_time(&mut self) -> bool {
        if !self.ran_out && Instant::now() >= self.setup.deadline {
            self.ran_out = true;
        }
        self.ran_out
    }

    /// `facts`' object, compiled once with the map's own argv: `None` when
    /// the compile failed, or made an object over [`MAX_OBJECT_BYTES`].
    fn object(&mut self, facts: &FileFacts) -> Result<Option<PathBuf>, Error> {
        if let Some(o) = self.objects.get(&facts.path) {
            return Ok(o.clone());
        }
        if self.out_of_time() {
            return Ok(None);
        }
        self.made += 1;
        let dir = self.fresh.0.join(format!("o{}", self.made));
        std::fs::create_dir(&dir).map_err(|e| Error::io(&dir, e))?;
        let object = dir.join("object.o");
        let deps = dir.join("object.d");
        let argv = compile_argv(
            &self.root,
            &self.setup.walked,
            self.setup.system_headers,
            facts,
            &facts.flags,
            &object,
            &deps,
        )?;
        let out = self.runner.tool_run(&argv);
        let _ = std::fs::remove_file(&deps);
        let made = match out {
            Ok(out) => matches!(out.end, ChildEnd::Exited(s) if s.success()),
            Err(_) => false,
        };
        let small = std::fs::metadata(&object).is_ok_and(|m| m.len() <= MAX_OBJECT_BYTES);
        let o = if made && small {
            Some(object)
        } else {
            let _ = std::fs::remove_file(&object);
            None
        };
        self.objects.insert(facts.path.clone(), o.clone());
        Ok(o)
    }

    /// Link `objects` with `extra` arguments; true when the linker exited 0.
    /// The program is deleted at once: nothing built is run.
    fn link_status(&mut self, objects: &[PathBuf], extra: &[String]) -> Result<bool, Error> {
        if self.out_of_time() {
            return Ok(false);
        }
        self.links += 1;
        let program = self.fresh.0.join(format!("program{}", self.links));
        let mut argv: Vec<String> = vec!["cc".into(), "-o".into(), path_arg(&program)?];
        if self.apple {
            // An object's `.linker_option` (`-lz`) never picks a library.
            argv.push("-Wl,-ignore_auto_link".into());
        }
        for o in objects {
            argv.push(path_arg(o)?);
        }
        argv.extend(extra.iter().cloned());
        let out = self.runner.tool_run(&argv);
        let _ = std::fs::remove_file(&program);
        Ok(matches!(out?.end, ChildEnd::Exited(s) if s.success()))
    }

    /// The trivial program's object, made once.
    fn probe(&mut self) -> Result<PathBuf, Error> {
        if let Some(p) = &self.probe {
            return Ok(p.clone());
        }
        let source = self.fresh.0.join("probe.c");
        std::fs::write(&source, "int main(void) { return 0; }\n")
            .map_err(|e| Error::io(&source, e))?;
        let object = self.fresh.0.join("probe.o");
        let argv = vec![
            "cc".into(),
            "-c".into(),
            "-w".into(),
            path_arg(&source)?,
            "-o".into(),
            path_arg(&object)?,
        ];
        self.runner
            .tool_outcome(&argv)?
            .map_err(|e| Error::Invariant(format!("the map's probe did not compile: {e}")))?;
        self.probe = Some(object.clone());
        Ok(object)
    }

    /// Whether zlib links on this system, asked once.
    fn zlib(&mut self) -> Result<bool, Error> {
        if let Some(z) = self.zlib {
            return Ok(z);
        }
        let probe = self.probe()?;
        let z = self.link_status(&[probe], &["-lz".into()])?;
        self.zlib = Some(z);
        Ok(z)
    }

    /// Whether the system provides every one of `syms` with `libs`.
    fn provides(&mut self, syms: &[&str], libs: &[String]) -> Result<bool, Error> {
        let probe = self.probe()?;
        let mut extra = Vec::new();
        for s in syms {
            // Identifier-shaped names only reach a linker flag.
            if !objsyms::identifier_shaped(s) || s.contains('$') {
                return Ok(false);
            }
            if self.apple {
                extra.push(format!("-Wl,-u,_{s}"));
            } else {
                extra.push(format!("-Wl,--require-defined={s}"));
            }
        }
        extra.extend(libs.iter().cloned());
        self.link_status(&[probe], &extra)
    }

    /// The symbols of `syms` the system does not provide, and those the
    /// probe budget (or the deadline) left undecided: group tests, halving
    /// a group that fails, at most [`MAX_PROBES`] links.
    fn missing(
        &mut self,
        syms: &[String],
        libs: &[String],
    ) -> Result<(Vec<String>, Vec<String>), Error> {
        let mut missing = Vec::new();
        let mut not_checked = Vec::new();
        let mut budget = MAX_PROBES;
        let mut stack: Vec<Vec<&str>> = vec![syms.iter().map(String::as_str).collect()];
        while let Some(group) = stack.pop() {
            if group.is_empty() {
                continue;
            }
            if budget == 0 || self.out_of_time() {
                not_checked.extend(group.iter().map(|s| (*s).to_string()));
                continue;
            }
            budget -= 1;
            if self.provides(&group, libs)? {
                continue;
            }
            if group.len() == 1 {
                // A link that failed only because the deadline came is no
                // proof the name is missing.
                if self.ran_out {
                    not_checked.push(group[0].to_string());
                } else {
                    missing.push(group[0].to_string());
                }
                continue;
            }
            let (a, b) = group.split_at(group.len() / 2);
            stack.push(b.to_vec());
            stack.push(a.to_vec());
        }
        missing.sort();
        not_checked.sort();
        Ok((missing, not_checked))
    }
}

impl Linker for CcLinker<'_> {
    fn link(&mut self, files: &[&FileFacts], unresolved: &[String]) -> Result<Linked, Error> {
        let zlib = if unresolved
            .iter()
            .any(|s| s.starts_with("deflate") || s.starts_with("inflate"))
        {
            self.zlib()?
        } else {
            false
        };
        let libs: Vec<String> = guess_libs(unresolved, zlib, self.apple)
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut objects = Vec::new();
        let mut not_compiled = Vec::new();
        for f in files {
            match self.object(f)? {
                Some(o) => objects.push(o),
                None => not_compiled.push(f.path.clone()),
            }
        }
        if not_compiled.is_empty() && self.link_status(&objects, &libs)? {
            return Ok(Linked::Ok);
        }
        if self.ran_out {
            not_compiled.clear();
        }
        let doubled: Vec<String> = closure::doubled(files).into_keys().collect();
        let (missing, not_checked) = self.missing(unresolved, &libs)?;
        not_compiled.sort();
        Ok(Linked::Failed {
            missing,
            doubled,
            not_checked,
            not_compiled,
        })
    }

    fn release(&mut self, path: &str) {
        if let Some(Some(object)) = self.objects.get(path) {
            let _ = std::fs::remove_file(object);
            if let Some(dir) = object.parent() {
                let _ = std::fs::remove_dir(dir);
            }
            // Recorded as released; asked again, it is compiled again.
            self.objects.remove(path);
        }
    }

    fn stopped(&self) -> bool {
        self.ran_out
    }
}

/// The analysis with its link checks: [`closure::analyze`] with a
/// [`CcLinker`] over `root`, compiling as the map did (`setup`). `None`
/// when the deadline passed before every link check ran.
pub fn analyze_linked(
    root: &Path,
    input: &Input<'_>,
    setup: LinkSetup<'_>,
) -> Result<Option<Analysis>, Error> {
    let mut linker = CcLinker::new(root, setup)?;
    let analysis = closure::analyze(input, Some(&mut linker))?;
    Ok((!linker.stopped()).then_some(analysis))
}

#[cfg(test)]
mod tests;
