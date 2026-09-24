//! `harness-tui` — the review cockpit's terminal front end
//! (docs/TUI-DESIGN.md §3–§4). Not built yet: the read model it will render
//! is the library (`harness_tui::model`).

#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    eprintln!(
        "harness-tui: the terminal front end is not built yet (docs/TUI-DESIGN.md §8 step 4); \
         the read model is the library `harness_tui::model`"
    );
    std::process::ExitCode::from(2)
}
