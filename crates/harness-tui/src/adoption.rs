//! The cockpit's adoption dialog (docs/PROJECT-MAP-DESIGN.md §3.7): a
//! target whose ledger was made elsewhere is asked about once, before the
//! terminal is taken — the same question as the CLI's `--adopt`, saying what
//! adopting does. Only a typed `y` adopts; an empty line (a held Enter),
//! anything else or the end of input leaves the folder untouched.

use std::io::{BufRead, Write};
use std::path::Path;

/// What adopting does, in the dialog's words.
pub fn explanation(adoption_file: &str) -> String {
    format!(
        "Adopting trusts these results on this computer from now on:\n\
         \x20 - the folder and a random token are recorded in {adoption_file}\n\
         \x20   (the token is also written to migration/.ruharness-adopted);\n\
         \x20 - the build folders that came with them are deleted: migration/build/, each unit\n\
         \x20   crate's target/, each attempt's candidate/target/ and every .promote-*/ —\n\
         \x20   nothing else;\n\
         \x20 - the verdicts stay claims made elsewhere until `harness verify` runs them here.\n"
    )
}

/// Ask whether to adopt the target at `target`, refused with `refusal`
/// (the sentence), on `input`/`output`; adopt it on a `y`. `Ok(true)` when
/// the folder is now trusted; `Ok(false)` when the person declined; `Err`
/// is a reason in words (the project's own `migration/`, an unwritable
/// adoption file).
pub fn ask(
    target: &Path,
    refusal: &str,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<bool, String> {
    let file = harness_core::adopt::adoption_file()
        .map(|p| p.display().to_string())
        .map_err(|e| e.to_string())?;
    let say = |output: &mut dyn Write, text: &str| {
        output
            .write_all(text.as_bytes())
            .and_then(|()| output.flush())
            .map_err(|e| format!("the terminal: {e}"))
    };
    say(
        output,
        &format!(
            "harness-tui: {}: {refusal}.\n\n{}\nAdopt this folder? Type y and Enter to adopt; \
             anything else leaves it untouched: ",
            target.display(),
            explanation(&file)
        ),
    )?;
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|e| format!("the terminal: {e}"))?;
    if !matches!(line.trim(), "y" | "Y") {
        say(output, "not adopted; nothing was changed\n")?;
        return Ok(false);
    }
    let done = harness_core::adopt::adopt(target).map_err(|e| e.to_string())?;
    for l in done.describe() {
        say(output, &format!("{l}\n"))?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_says_what_adopting_does_and_adopts_only_on_y() {
        harness_core::adopt::testing::adoption_file();
        let dir = std::env::temp_dir().join(format!(
            "harness-tui-adoption-{}-{}",
            std::process::id(),
            harness_core::hash::random_hex(4)
        ));
        let m = dir.join("migration");
        std::fs::create_dir_all(m.join("build/x")).unwrap();
        std::fs::write(m.join("plan.toml"), "schema_version = 1\n").unwrap();
        let refusal = harness_core::adopt::check(&dir).unwrap_err().to_string();
        for declined in ["\n", "yes please\n", ""] {
            let mut out = Vec::new();
            let adopted = ask(&dir, &refusal, &mut declined.as_bytes(), &mut out).unwrap();
            assert!(!adopted);
            let text = String::from_utf8(out).unwrap();
            assert!(
                text.contains("made elsewhere (0 units, 0 verified)"),
                "{text}"
            );
            assert!(text.contains("the build folders that came with them are deleted"));
            assert!(text.contains("until `harness verify` runs them here"));
            assert!(harness_core::adopt::check(&dir).is_err());
            assert!(m.join("build").exists());
        }
        let mut out = Vec::new();
        assert!(ask(&dir, &refusal, &mut "y\n".as_bytes(), &mut out).unwrap());
        harness_core::adopt::check(&dir).unwrap();
        assert!(!m.join("build").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
