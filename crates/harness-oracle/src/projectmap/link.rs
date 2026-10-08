//! The map's link checks (docs/PROJECT-MAP-DESIGN.md §3.5): the real
//! [`Linker`]. Each file of a closure is compiled once per run (with its own
//! include folders and the configuration's flags) into a fresh folder under
//! the map's sandboxed runner, and the objects are linked into one program
//! with the outside libraries guessed from the outside symbols. Nothing
//! built is ever run.
//!
//! The result comes from the symbol facts and the linker's exit status,
//! never from its text (which can carry project text): when a link fails,
//! `doubled` is every symbol two linked files define strongly, and
//! `missing` every unresolved symbol the system's libraries do not provide,
//! found by linking a trivial program that requires a group of them
//! (`-u` on Mach-O, `--require-defined` elsewhere) and halving a group that
//! fails.

use super::closure::{self, Analysis, Input, Linked, Linker};
use super::{FileFacts, Fresh, MAP_CFLAGS};
use crate::exec::{ChildEnd, Runner};
use crate::objsyms;
use harness_core::error::Error;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The wall-clock limit of one compile or link (§3.10).
pub const LINK_TIMEOUT_SECS: u64 = 120;
/// The most probe links one failed link may spend finding its missing
/// symbols; symbols still undecided past it are reported missing.
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

/// What every compile and link of one run shares.
struct LinkCtx {
    root: PathBuf,
    runner: Runner,
}

fn path_arg(p: &Path) -> Result<String, Error> {
    crate::path_str(p).map(str::to_string)
}

/// Compile one `.c` to `<out_dir>/object.o` with the judge's base flags,
/// the configuration's `flags`, then the file's own include folders:
/// `Ok(None)` when the compile failed (a fact, not an error).
fn compile_object(
    ctx: &LinkCtx,
    facts: &FileFacts,
    flags: &[String],
    out_dir: &Path,
) -> Result<Option<PathBuf>, Error> {
    let object = out_dir.join("object.o");
    let mut argv: Vec<String> = vec!["cc".into(), "-c".into(), "-w".into()];
    argv.extend(MAP_CFLAGS.iter().map(|f| (*f).to_string()));
    argv.extend(flags.iter().cloned());
    for dir in &facts.include_dirs {
        argv.push(format!("-I{}", path_arg(&ctx.root.join(dir))?));
    }
    argv.push(path_arg(&ctx.root.join(&facts.path))?);
    argv.extend(["-o".into(), path_arg(&object)?]);
    let out = ctx.runner.tool_run(&argv)?;
    Ok(match out.end {
        ChildEnd::Exited(status) if status.success() => Some(object),
        _ => {
            let _ = std::fs::remove_file(&object);
            None
        }
    })
}

/// The real linker: `cc` under the map's runner, every object and program
/// in its own fresh folder, removed with it.
pub struct CcLinker {
    ctx: LinkCtx,
    flags: Vec<String>,
    fresh: Fresh,
    apple: bool,
    /// Objects by file path (`None`: the compile failed).
    objects: BTreeMap<String, Option<PathBuf>>,
    /// The trivial program the probes link.
    probe: Option<PathBuf>,
    zlib: Option<bool>,
    links: usize,
}

impl CcLinker {
    /// A linker for the project at `root` (canonical), compiling with the
    /// configuration's `flags`.
    pub fn new(root: &Path, flags: &[String]) -> Result<CcLinker, Error> {
        CcLinker::new_in(root, flags, &std::env::temp_dir())
    }

    /// [`CcLinker::new`] with the fresh folder made under `fresh_parent`.
    pub(crate) fn new_in(
        root: &Path,
        flags: &[String],
        fresh_parent: &Path,
    ) -> Result<CcLinker, Error> {
        let root = root.canonicalize().map_err(|e| Error::io(root, e))?;
        let fresh = Fresh::make(fresh_parent)?;
        let runner = Runner::map(
            &root,
            &fresh.0,
            &["cc"],
            Duration::from_secs(LINK_TIMEOUT_SECS),
        )?;
        Ok(CcLinker {
            ctx: LinkCtx { root, runner },
            flags: flags.to_vec(),
            fresh,
            apple: cfg!(target_vendor = "apple"),
            objects: BTreeMap::new(),
            probe: None,
            zlib: None,
            links: 0,
        })
    }

    fn object(&mut self, facts: &FileFacts) -> Result<Option<PathBuf>, Error> {
        if let Some(o) = self.objects.get(&facts.path) {
            return Ok(o.clone());
        }
        let dir = self.fresh.0.join(format!("o{}", self.objects.len()));
        std::fs::create_dir(&dir).map_err(|e| Error::io(&dir, e))?;
        let o = compile_object(&self.ctx, facts, &self.flags, &dir)?;
        self.objects.insert(facts.path.clone(), o.clone());
        Ok(o)
    }

    /// Link `objects` with `extra` arguments; true when the linker exited 0.
    /// The program is deleted at once: nothing built is run.
    fn link_status(&mut self, objects: &[PathBuf], extra: &[String]) -> Result<bool, Error> {
        self.links += 1;
        let program = self.fresh.0.join(format!("program{}", self.links));
        let mut argv: Vec<String> = vec!["cc".into(), "-o".into(), path_arg(&program)?];
        for o in objects {
            argv.push(path_arg(o)?);
        }
        argv.extend(extra.iter().cloned());
        let out = self.ctx.runner.tool_run(&argv);
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
        self.ctx
            .runner
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

    /// The symbols of `syms` the system does not provide: group tests,
    /// halving a group that fails, at most [`MAX_PROBES`] links.
    fn missing(&mut self, syms: &[String], libs: &[String]) -> Result<Vec<String>, Error> {
        let mut missing = Vec::new();
        let mut budget = MAX_PROBES;
        let mut stack: Vec<Vec<&str>> = vec![syms.iter().map(String::as_str).collect()];
        while let Some(group) = stack.pop() {
            if group.is_empty() {
                continue;
            }
            if budget == 0 {
                missing.extend(group.iter().map(|s| (*s).to_string()));
                continue;
            }
            budget -= 1;
            if self.provides(&group, libs)? {
                continue;
            }
            if group.len() == 1 {
                missing.push(group[0].to_string());
                continue;
            }
            let (a, b) = group.split_at(group.len() / 2);
            stack.push(b.to_vec());
            stack.push(a.to_vec());
        }
        missing.sort();
        Ok(missing)
    }
}

impl Linker for CcLinker {
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
        let mut compiled = true;
        for f in files {
            match self.object(f)? {
                Some(o) => objects.push(o),
                None => compiled = false,
            }
        }
        if compiled && self.link_status(&objects, &libs)? {
            return Ok(Linked::Ok);
        }
        let doubled: Vec<String> = closure::doubled(files).into_keys().collect();
        let missing = self.missing(unresolved, &libs)?;
        Ok(Linked::Failed { missing, doubled })
    }
}

/// The analysis with its link checks: [`closure::analyze`] with a
/// [`CcLinker`] over `root`, compiling with the configuration's `flags`.
pub fn analyze_linked(root: &Path, input: &Input<'_>, flags: &[String]) -> Result<Analysis, Error> {
    let mut linker = CcLinker::new(root, flags)?;
    closure::analyze(input, Some(&mut linker))
}

#[cfg(test)]
mod tests;
