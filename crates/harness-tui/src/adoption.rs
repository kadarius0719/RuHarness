//! The cockpit's adoption dialog (docs/PROJECT-MAP-DESIGN.md §3.7): a
//! target whose ledger was made elsewhere is asked about once, before the
//! terminal is taken — the same question as the CLI's `--adopt`, saying what
//! adopting does. Only a typed `y` adopts; an empty line (a held Enter),
//! anything else or the end of input leaves the folder untouched.

use std::io::{BufRead, Write};
use std::path::Path;

/// The question in three plain lines — the start's and the in-cockpit
/// dialog's alike (the whole-journey check, item 11): what the folder
/// holds, what adopting trusts, what it removes. The rest is the card.
pub fn short_words() -> Vec<String> {
    vec![
        "This folder holds migration results made on another computer, or in another copy of \
         the project."
            .into(),
        "Adopting trusts them here: the harness may build and run the code they hold, in its \
         sandbox, and checks each result again before it counts."
            .into(),
        "It deletes the build folders that came with them; nothing else changes.".into(),
    ]
}

/// What adopting does, in full: the card behind the question (`i` in the
/// cockpit, `i` and Enter at the start). `refusal`: the command's own
/// words for the folder's claim, when known.
pub fn card_words(adoption_file: &str, refusal: Option<&str>) -> Vec<String> {
    let mut card = vec![
        "Adopting trusts these results in this folder on this computer from now on; a copy or \
         another checkout of the project is asked again."
            .to_string(),
        "The harness will build and run the code they hold — test programs, Rust, features and \
         workloads — in the sandbox."
            .into(),
        format!(
            "The folder, a fresh random token and the time are recorded in {adoption_file} (the \
             token is also written to migration/.ruharness-adopted, replacing any that came with \
             the folder)."
        ),
        "The build folders that came with them are deleted, in migration/ and in each tool's \
         under migration/tools/: build/, each piece's Rust target/, each try's candidate/target/ \
         and every .promote-*/ — nothing else."
            .into(),
        "Each check result that came with them is marked \"made elsewhere\" (in the tree and in \
         `harness state status`) until `harness verify` runs it here."
            .into(),
    ];
    if let Some(r) = refusal {
        card.push(format!("In detail: {r}."));
    }
    card
}

/// The card as the terminal prints it: each line an indented dash item.
fn card_text(adoption_file: &str, refusal: &str) -> String {
    card_words(adoption_file, Some(refusal))
        .iter()
        .map(|l| format!("  - {l}\n"))
        .collect()
}

/// The start's question after the three lines.
const QUESTION: &str = "Adopt this folder? Type y and Enter to adopt, i and Enter for what it \
                        does in full; anything else leaves it untouched: ";

/// Ask whether to adopt the target at `target`, whose ledger `refusal`
/// describes (the folder and what it claims, [`harness_core::adopt::
/// made_elsewhere`]), on `input`/`output`; adopt it on a `y`. `i` prints
/// the card and asks again. `Ok(true)` when the folder is now trusted;
/// `Ok(false)` when the person declined; `Err` is a reason in words (the
/// project's own `migration/`, an unwritable adoption file). `tool` is the
/// mapped tool the cockpit opens, for the commands the adoption lines name.
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
        &format!("{}\n\n{QUESTION}", short_words().join("\n")),
    )?;
    let mut line = String::new();
    loop {
        line.clear();
        input
            .read_line(&mut line)
            .map_err(|e| format!("the terminal: {e}"))?;
        if matches!(line.trim(), "i" | "I") {
            say(
                output,
                &format!("\n{}\n{QUESTION}", card_text(&file, refusal)),
            )?;
            continue;
        }
        break;
    }
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
        for declined in ["\n", "yes please\n", "", "i\nno\n"] {
            let mut out = Vec::new();
            let adopted = ask(&dir, None, &refusal, &mut declined.as_bytes(), &mut out).unwrap();
            assert!(!adopted);
            let text = String::from_utf8(out).unwrap();
            // Three plain lines first, then the question (the whole-journey
            // check, item 11): no "ledger", no "verdict".
            assert!(
                text.starts_with(&format!(
                    "{}\n\nAdopt this folder?",
                    short_words().join("\n")
                )),
                "{text}"
            );
            assert_eq!(short_words().len(), 3);
            for word in ["ledger", "verdict"] {
                assert!(!text.contains(word), "{word}: {text}");
            }
            // Only the cockpit's own way: no `--adopt` to add.
            assert!(!text.contains("--adopt"), "{text}");
            // The rest is the card, printed on `i`.
            let card = declined.starts_with('i');
            assert_eq!(
                text.contains(&format!(
                    "{}: this folder already holds migration results made elsewhere (0 units, \
                     0 verified).",
                    root.display()
                )),
                card,
                "{text}"
            );
            if card {
                assert!(
                    text.contains(
                        "the code they hold — test programs, Rust, features and workloads — in \
                         the sandbox"
                    ),
                    "{text}"
                );
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
            }
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
