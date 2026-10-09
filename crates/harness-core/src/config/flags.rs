//! The flag grammar (docs/PROJECT-MAP-DESIGN.md §3.2): the only compiler
//! flags a configuration may carry, whoever proposed them — the person's
//! `harness.toml`, a `compile_commands.json`, a model. Every target file is
//! hostile input, so a flag outside the grammar is refused by name.
//!
//! Only the joined forms are read (`-Iinclude`, never `-I include`): one
//! flag is one argument, so nothing can hide in the next one.

/// The `-std=` values a configuration may name.
pub const STD_VALUES: &[&str] = &[
    "c89", "c90", "c99", "c11", "c17", "c18", "c23", "c2x", "gnu89", "gnu90", "gnu99", "gnu11",
    "gnu17", "gnu18", "gnu23", "gnu2x",
];

/// The `-f` flags a configuration may carry: an exact list of spellings,
/// extended only in RuHarness's own code, never in a target file.
pub const F_FLAGS: &[&str] = &[
    "-fno-strict-aliasing",
    "-fwrapv",
    "-fno-common",
    "-fcommon",
    "-fPIC",
    "-fpic",
    "-fsigned-char",
    "-funsigned-char",
    "-fno-builtin",
    "-fvisibility=hidden",
    "-fvisibility=default",
];

/// The path-taking flags (`-isystem`, `-include`, `-iquote` and
/// `-idirafter` all start with `-i` and none is a prefix of another).
pub const PATH_FLAGS: &[&str] = &["-isystem", "-include", "-iquote", "-idirafter", "-I"];

/// A path flag split into its prefix (one of [`PATH_FLAGS`]) and its path
/// as written; `None` for any other flag. Every reader that places a path
/// flag's folder takes the prefix from here.
pub fn split_path_flag(flag: &str) -> Option<(&'static str, &str)> {
    PATH_FLAGS
        .iter()
        .find_map(|p| flag.strip_prefix(p).map(|rest| (*p, rest)))
}

/// What a flag of the grammar is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag<'a> {
    /// `-D<name>` or `-D<name>=<value>`.
    Define,
    /// `-U<name>`.
    Undefine,
    /// `-I`, `-iquote`, `-isystem`, `-idirafter` or `-include` with its
    /// path, as written
    /// (`.` or a clean path relative to the project root, never under
    /// `migration/`); the caller resolves it against the root.
    Path(&'a str),
    /// `-std=`, `-pthread` or one of [`F_FLAGS`].
    Plain,
    /// `-O0`–`-O3`: recorded, never applied (every compile keeps its own).
    Optimization,
}

/// Check one flag against the grammar. `Err` is one plain sentence naming
/// the flag and what to do.
pub fn check_flag(flag: &str) -> Result<Flag<'_>, String> {
    let shown = crate::text::safe_line(flag);
    if let Some(rest) = flag.strip_prefix("-D") {
        let (name, value) = match rest.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (rest, None),
        };
        if !is_identifier(name) {
            return Err(format!(
                "the flag `{shown}` does not define a C identifier; write -DNAME or -DNAME=VALUE \
                 as one argument"
            ));
        }
        if let Some(v) = value {
            no_option_value(flag, v)?;
        }
        return Ok(Flag::Define);
    }
    if let Some(name) = flag.strip_prefix("-U") {
        if !is_identifier(name) {
            return Err(format!(
                "the flag `{shown}` does not undefine a C identifier; write -UNAME"
            ));
        }
        return Ok(Flag::Undefine);
    }
    for prefix in PATH_FLAGS {
        if let Some(path) = flag.strip_prefix(prefix) {
            if path.is_empty() {
                return Err(format!(
                    "the flag `{shown}` has no path joined to it; write it as one argument, like \
                     {prefix}include"
                ));
            }
            no_option_value(flag, path)?;
            if !inside_root_lexically(path) {
                return Err(format!(
                    "the flag `{shown}` names a path outside the project or under migration/; \
                     name a folder inside the project, relative to its root"
                ));
            }
            return Ok(Flag::Path(path));
        }
    }
    if let Some(std) = flag.strip_prefix("-std=") {
        if STD_VALUES.contains(&std) {
            return Ok(Flag::Plain);
        }
        return Err(format!(
            "the flag `{shown}` names a C standard the harness does not know; use one of {}",
            STD_VALUES.join(" ")
        ));
    }
    if flag == "-pthread" || F_FLAGS.contains(&flag) {
        return Ok(Flag::Plain);
    }
    if matches!(flag, "-O0" | "-O1" | "-O2" | "-O3") {
        return Ok(Flag::Optimization);
    }
    if is_bookkeeping(flag) {
        return Err(format!(
            "the flag `{shown}` is added by the harness itself: remove it"
        ));
    }
    Err(format!(
        "the flag `{shown}` is not one the harness passes to a compiler; remove it (the flags \
         allowed are listed in docs/SCHEMAS.md, \"harness.toml, file-list form\")"
    ))
}

/// A flag the harness adds to every compile itself (`-c`, `-o <object>`,
/// `-MD -MF <deps>`, joined or not): a configuration never carries it.
fn is_bookkeeping(flag: &str) -> bool {
    matches!(flag, "-c" | "-MD") || flag.starts_with("-o") || flag.starts_with("-MF")
}

/// A value a compiler would read as another option (`-…`) or as a file of
/// options (`@file`) is refused.
fn no_option_value(flag: &str, value: &str) -> Result<(), String> {
    if value.starts_with(['@', '-']) {
        return Err(format!(
            "the flag `{}` has a value starting with `@` or `-`, which a compiler reads as more \
             options; remove it",
            crate::text::safe_line(flag)
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(format!(
            "the flag `{}` holds a control character; remove it",
            crate::text::safe_line(flag)
        ));
    }
    Ok(())
}

/// A C identifier: `[A-Za-z_][A-Za-z0-9_]*`.
pub fn is_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// `.` (the project root itself) or a clean path relative to the root
/// whose first part is not `migration` — the lexical half of "inside the
/// project root and not under `migration/`"; [`super::TargetConfig::load`]
/// resolves links for the other half.
pub fn inside_root_lexically(path: &str) -> bool {
    let path = path.strip_suffix('/').unwrap_or(path);
    path == "."
        || (crate::plan::is_clean_relative_path(path)
            && path.split('/').next() != Some(crate::ledger::MIGRATION_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grammar_takes_its_flags() {
        for ok in [
            "-DLZ4IO_MULTITHREAD",
            "-DX=1",
            "-D_GNU_SOURCE",
            "-DNAME=a b",
            "-UNDEBUG",
            "-Iinclude",
            "-I.",
            "-Isrc/include/",
            "-iquotesrc",
            "-isystemvendor/inc",
            "-idiraftercompat",
            "-includeconfig.h",
            "-std=c99",
            "-std=gnu2x",
            "-pthread",
            "-fwrapv",
            "-fvisibility=hidden",
            "-O0",
            "-O3",
        ] {
            assert!(check_flag(ok).is_ok(), "{ok}: {:?}", check_flag(ok));
        }
        assert_eq!(check_flag("-Iinc").unwrap(), Flag::Path("inc"));
        assert_eq!(check_flag("-O2").unwrap(), Flag::Optimization);
        assert_eq!(
            check_flag("-idiraftercompat").unwrap(),
            Flag::Path("compat")
        );
        assert_eq!(
            split_path_flag("-idiraftercompat"),
            Some(("-idirafter", "compat"))
        );
        assert_eq!(split_path_flag("-Iinc"), Some(("-I", "inc")));
        assert_eq!(split_path_flag("-DX"), None);
    }

    #[test]
    fn a_flag_outside_the_grammar_is_refused_by_name() {
        for (bad, says) in [
            ("-fuse-ld=/x", "is not one the harness passes"),
            ("-I@f", "starting with `@` or `-`"),
            ("-I-x", "starting with `@` or `-`"),
            ("-DFOO BAR", "does not define a C identifier"),
            ("-D1X", "does not define a C identifier"),
            ("-DX=@file", "starting with `@` or `-`"),
            ("-DX=-O", "starting with `@` or `-`"),
            ("-I../outside", "outside the project or under migration/"),
            ("-I/usr/include", "outside the project or under migration/"),
            (
                "-Imigration/tools/t-x",
                "outside the project or under migration/",
            ),
            (
                "-includemigration/x.h",
                "outside the project or under migration/",
            ),
            ("-I", "has no path joined"),
            ("-idirafter", "has no path joined"),
            (
                "-idirafter/usr/include",
                "outside the project or under migration/",
            ),
            (
                "-idiraftermigration/x",
                "outside the project or under migration/",
            ),
            ("-std=c++17", "C standard"),
            ("-O", "is not one"),
            ("-Os", "is not one"),
            ("-B/x", "is not one"),
            ("-fplugin=x.so", "is not one"),
            ("-Xclang", "is not one"),
            ("-Wl,-rpath,x", "is not one"),
            ("-MD", "is added by the harness itself: remove it"),
            ("-MFdeps.d", "is added by the harness itself: remove it"),
            ("-o", "is added by the harness itself: remove it"),
            ("-oout.o", "is added by the harness itself: remove it"),
            ("-c", "is added by the harness itself: remove it"),
            ("-x", "is not one"),
            ("@args", "is not one"),
            ("-lfoo", "is not one"),
            ("-DX=a\nb", "control character"),
        ] {
            let err = check_flag(bad).expect_err(bad);
            assert!(err.contains(says), "{bad}: {err}");
            assert!(err.starts_with("the flag `"), "{bad}: {err}");
        }
    }
}
