//! `harness-mcp` — a stdio MCP server exposing a target's migration ledger
//! as structured data and the review acts that stay labelled
//! (docs/MCP-DESIGN.md). Everything it knows it reads with
//! `harness_tui::model`; everything it writes is a spawned `harness --json …`
//! through `harness_tui::spawn`, so the writer lock, the sandbox, the oracle
//! and the ledger's rules apply unchanged; it writes no file itself (an
//! answer to a hand-off it posed goes to the CLI's stdin). With `--cockpit`
//! it is the server of the cockpit's chat: no harness binary at all, reads,
//! and act tools that only ask (docs/CHAT-PANE-DESIGN.md §4.4).

#![forbid(unsafe_code)]

mod acts;
mod policy;
mod reads;
mod rpc;
mod server;
mod tools;

/// The fence lives in the read model's crate: one implementation for this
/// server and the cockpit's chat (docs/CHAT-PANE-DESIGN.md §3.3).
use harness_tui::fence;

use harness_tui::spawn::ChildSlot;
use server::{Gate, Input, Server};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

fn main() {
    // First: a signal must never leave a child running on the writer lock.
    let slot = ChildSlot::default();
    let gate = Gate::default();
    let mut signals = match Signals::new([SIGINT, SIGTERM, SIGHUP]) {
        Ok(s) => s,
        Err(e) => {
            server::log(&format!("cannot install signal handlers: {e}"));
            std::process::exit(1);
        }
    };
    // A panic on any thread interrupts the child, without blocking (the
    // panicking thread may hold the gate or the slot), and ends the server
    // (§R2 PROTO-2, VB-5): one that lost a thread is not left half alive.
    {
        let (slot, gate) = (slot.clone(), gate.clone());
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            server::shut_down_now(&gate, &slot);
            default(info);
            std::process::exit(101);
        }));
    }
    {
        let (slot, gate) = (slot.clone(), gate.clone());
        std::thread::spawn(move || {
            if let Some(sig) = signals.forever().next() {
                server::shut_down(&gate, &slot);
                // Die BY the signal, as the CLI does; exit(128+N) only if the
                // re-raise fails.
                let _ = signal_hook::low_level::emulate_default_handler(sig);
                std::process::exit(128 + sig);
            }
        });
    }
    let args: Vec<String> = match std::env::args_os()
        .skip(1)
        .map(|a| a.into_string())
        .collect::<Result<_, _>>()
    {
        Ok(args) => args,
        Err(bad) => {
            server::log(&format!(
                "argument {bad:?} is not UTF-8\n{}",
                policy::usage()
            ));
            std::process::exit(2);
        }
    };
    let cfg = match policy::parse_args(&args) {
        Ok(Some(cfg)) => cfg,
        Ok(None) => std::process::exit(0),
        Err(e) => {
            server::log(&e);
            std::process::exit(2);
        }
    };
    server::log(&format!(
        "serving {} (roots: {}; providers: {}; harness: {}{})",
        cfg.target.display(),
        cfg.target_roots.len(),
        cfg.providers.join(", "),
        match (&cfg.harness, cfg.cockpit) {
            (_, true) => "none — the cockpit's server: it runs no act".to_string(),
            (None, false) => "none — read-only".to_string(),
            (Some(h), false) => h.display().to_string(),
        },
        if cfg.allow_unsandboxed {
            "; UNSANDBOXED"
        } else {
            ""
        }
    ));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        loop {
            let input = match rpc::read_frame(&mut reader, rpc::MAX_LINE_BYTES) {
                Ok(Some(frame)) => Input::Frame(frame),
                Ok(None) => Input::Eof,
                Err(e) => Input::ReadError(e.to_string()),
            };
            let last = !matches!(input, Input::Frame(_));
            if tx.send(input).is_err() || last {
                return;
            }
        }
    });
    let stdout = std::io::stdout();
    let code = Server::new(cfg, stdout.lock(), slot, gate).run(rx);
    std::process::exit(code);
}
