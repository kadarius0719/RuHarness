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
        "Adopting trusts these results in this folder on this computer from now on (a copy\n\
         \x20 or another checkout of the project is asked again):\n\
         \x20 - the harness will build and run the code they hold — drivers, Rust crates,\n\
         \x20   features and workloads — in the sandbox;\n\
         \x20 - the folder, a fresh random token and the time are recorded in {adoption_file}\n\
         \x20   (the token is also written to migration/.ruharness-adopted, replacing any\n\
         \x20   that came with the folder);\n\
         \x20 - the build folders that came with them are deleted, in the project's ledger and\n\
         \x20   in each tool's under migration/tools/: build/, each unit crate's target/, each\n\
         \x20   attempt's candidate/target/ and every .promote-*/ — nothing else;\n\
         \x20 - each verdict that came with them is marked \"made elsewhere\" (in the tree and in\n\
         \x20   `harness state status`) until `harness verify` runs it here.\n"
    )
}

/// Ask whether to adopt the target at `target`, whose ledger `refusal`
/// describes (the folder and what it claims, [`harness_core::adopt::
/// made_elsewhere`]), on `input`/`output`; adopt it on a `y`. `Ok(true)` when
/// the folder is now trusted; `Ok(false)` when the person declined; `Err`
/// is a reason in words (the project's own `migration/`, an unwritable
/// adoption file). `tool` is the mapped tool the cockpit opens, for the
/// commands the adoption lines name.
pub fn ask(
    target: &Path,
    tool: Option<&str>,
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
            "harness-tui: {refusal}.\n\n{}\nAdopt this folder? Type y and Enter to adopt; \
             anything else leaves it untouched: ",
            explanation(&file)
        ),
    )?;
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|e| format!("the terminal: {e}"))?;
    if !matches!(line.trim(), "y" | "Y") {
        say(
            output,
            "not adopted; nothing was changed — start the cockpit again and type y when you \
             want to adopt it\n",
        )?;
        return Ok(false);
    }
    let done = harness_core::adopt::adopt(target).map_err(|e| e.to_string())?;
    for l in done.describe(tool) {
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
        let harness_core::Error::NotAdopted {
            root,
            units,
            verified,
        } = harness_core::adopt::check(&dir).unwrap_err()
        else {
            panic!("not the refusal");
        };
        let refusal = harness_core::adopt::made_elsewhere(&root, units, verified);
        for declined in ["\n", "yes please\n", ""] {
            let mut out = Vec::new();
            let adopted = ask(&dir, None, &refusal, &mut declined.as_bytes(), &mut out).unwrap();
            assert!(!adopted);
            let text = String::from_utf8(out).unwrap();
            assert!(
                text.contains(&format!(
                    "{}: this folder already holds migration results made elsewhere (0 units, \
                     0 verified).",
                    root.display()
                )),
                "{text}"
            );
            // Only the cockpit's own way: no `--adopt` to add.
            assert!(!text.contains("--adopt"), "{text}");
            assert!(
                text.contains("the harness will build and run the code they hold")
                    && text.contains("in the sandbox"),
                "{text}"
            );
            assert!(text.contains("the build folders that came with them are deleted"));
            // Each tool's build folders are named too.
            assert!(
                text.contains("in each tool's under migration/tools/"),
                "{text}"
            );
            assert!(
                text.contains("is marked \"made elsewhere\" (in the tree and in"),
                "{text}"
            );
            assert!(
                text.contains("until `harness verify` runs it here"),
                "{text}"
            );
            assert!(text.contains("another checkout of the project is asked again"));
            // Declining names the way back.
            assert!(
                text.contains("start the cockpit again and type y when you want to adopt it"),
                "{text}"
            );
            assert!(harness_core::adopt::check(&dir).is_err());
            assert!(m.join("build").exists());
        }
        let mut out = Vec::new();
        assert!(ask(&dir, None, &refusal, &mut "y\n".as_bytes(), &mut out).unwrap());
        harness_core::adopt::check(&dir).unwrap();
        assert!(!m.join("build").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
