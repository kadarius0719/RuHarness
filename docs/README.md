# What every document is for

**Current** means it says what the program is today and is kept so. **History** means it was
written before the thing was built and is kept for the reasoning; its first lines say when it was
built and where the truth lives now. A new document gets a row here, or it is not finished.

| Document | Kind | What it is for |
|---|---|---|
| **People using RuHarness** | | |
| [../README.md](../README.md) | current | The front door: where to go, then the command reference for developers |
| [FIRST-HOUR.md](FIRST-HOUR.md) | current | From nothing to one piece of liblzg in Rust, proven by the judge, then broken on purpose |
| [topics/](topics/) | current | Seven short pages, one per part: [cockpit](topics/cockpit.md), [drivers](topics/drivers.md), [features](topics/features.md), [judge](topics/judge.md), [map](topics/map.md), [models](topics/models.md), [speed](topics/speed.md) |
| [TUTORIAL.md](TUTORIAL.md) | current | The ideas, in order, with a friendly reference appendix |
| [TESTING-GUIDE.md](TESTING-GUIDE.md) | current | Every feature tried by hand on liblzg, step by step (13 parts) |
| **Developers** | | |
| [SCHEMAS.md](SCHEMAS.md) | current | The contract: every ledger file, every command, every refusal the code cites |
| [FEATURES-DESIGN.md](FEATURES-DESIGN.md) | current | Features and scenarios; governs with SCHEMAS |
| [FEATURES-PROBE-REDESIGN.md](FEATURES-PROBE-REDESIGN.md) | current | The compiler-guided probe; replaces FEATURES-DESIGN §5.3–§5.4 |
| [PERF-DESIGN.md](PERF-DESIGN.md) | current | Speed (`harness perf`); governs with SCHEMAS; its Linux text is a plan |
| [PROJECT-MAP-DESIGN.md](PROJECT-MAP-DESIGN.md) | current | The project map; §8 holds what is open, the fuller C–Rust boundary among it |
| [BEYOND-LEAVES-DESIGN.md](BEYOND-LEAVES-DESIGN.md) | current | A piece that calls other pieces moves: the callees linked as the program stands; from the 2026-10-21 spike |
| [MCP-DESIGN.md](MCP-DESIGN.md) | current | harness-mcp's full tool contract |
| [CHAT-PANE-DESIGN.md](CHAT-PANE-DESIGN.md) | current | The cockpit's chat: its command line, its acts, the requester label |
| [COCKPIT-WRAPPER-DESIGN.md](COCKPIT-WRAPPER-DESIGN.md) | current | The cockpit's screen, menu and armed dialogs |
| [../devtools/README.md](../devtools/README.md) | current | Scripts for testing RuHarness itself: the cockpit driver, the map spike, the guide's gates |
| **Sessions** (in reading order) | | |
| [../CLAUDE.md](../CLAUDE.md) | current | What a session reads first, and the rules never skipped |
| [AGENT-BRIEFING.md](AGENT-BRIEFING.md) | current | The mission, the architecture, the engineering standards |
| [NEXT-SESSION.md](NEXT-SESSION.md) | current | Where the last session stopped and the work, in order |
| [../DECISIONS.md](../DECISIONS.md) | current | The engineering log, newest last: what was decided and why |
| **History** | | |
| [TUI-DESIGN.md](TUI-DESIGN.md) | history | The first cockpit; §2 and §4 still describe its engine |
| [M4-DESIGN.md](M4-DESIGN.md) | history | The TRACTOR benchmark and driver generation; reference: SCHEMAS "M4 additions" |
| [ORACLE-HARDENING.md](ORACLE-HARDENING.md) | history | The sanitized C pass and the boundary check; reference: SCHEMAS |
| [CLI-HARDENING.md](CLI-HARDENING.md) | history | The writer lock, `promote`, cancellation, `--json`; reference: SCHEMAS |
| [REPLAY-DESIGN.md](REPLAY-DESIGN.md) | history | Evidence-first replay; read its §R and §C first |
| [PROJECT-MAP-INVESTIGATION.md](PROJECT-MAP-INVESTIGATION.md) | history | The spike behind the map, with its measurements |
| [reviews/](reviews/) | history | Review and check records, one file each, dated; never edited |
