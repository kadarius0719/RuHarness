# Your first hour with RuHarness

RuHarness moves a C program to Rust one piece at a time, and a judge proves each piece behaves exactly like the C.
This page takes you from nothing to one piece of a real program (liblzg, a small compressor) in Rust, proven by the judge, then broken on purpose and caught.
Paste each box into the Terminal app (Applications → Utilities) whole and press Return; the line above each box says the one thing to look for.

## 1. Get the tools and RuHarness (the build about 2 minutes on an M3 Mac, the installs longer)

Install Apple's developer tools (they hold the C compiler and git); click **Install** in the window that opens and wait for it to finish.
```bash
xcode-select --install
```
Install Rust; press Return at its menu, wait for `Rust is installed now. Great!`, then open a new Terminal window.
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
Install jq (reads the harness's files); if `brew` is not found, first install Homebrew with the one command on https://brew.sh; its installer ends by asking you to run two more lines, and `brew` works only after them.
```bash
brew install jq
```
Download RuHarness and go into its folder.
```bash
git clone https://github.com/kadarius0719/RuHarness.git ~/code/RuHarness && cd ~/code/RuHarness
```
Build the `harness` program; it is done when the last line ends in `` (executable `harness`) ``.
```bash
cargo install --locked --path crates/harness-cli
```
Prove the tools work on the example that comes with RuHarness (about 20 seconds); the last line must read `verify: u001-katajainen GREEN — status set to verified`; the lines about trusting the folder are the harness asking once.
```bash
harness state status --target targets/zopfli --adopt && harness verify u001-katajainen --target targets/zopfli
```

## 2. Copy liblzg and let the harness find its programs (about 1 minute on an M3 Mac)

Download liblzg at the exact version this page was tested on; the last line reads `HEAD is now at 182b56c Bump version to 1.0.10`.
```bash
git clone https://gitlab.com/mbitsnbites/liblzg.git ~/code/liblzg-upstream && git -C ~/code/liblzg-upstream checkout --detach 182b56cb36843720f38eff2ec30db1deac4e85bd
```
Copy it into a folder of its own, make a folder for your drafts, and go into the copy; nothing prints.
```bash
rsync -a --exclude .git ~/code/liblzg-upstream/ ~/lzg-map/ && mkdir -p ~/lzg-practice && cd ~/lzg-map
```
Map it (the harness finds every program in the folder); look for `programs: 3` and, under `p2 t-lzg`, `link check: linked`; ignore `the choice is yours` for now (that is another program).
```bash
harness project map
```
Tell it how liblzg is built; this is one box down to `EOF`, and Terminal shows `heredoc>` while it pastes.
```bash
cat > migration/map/config.toml <<'EOF'
[[configuration]]
name = "make"
from = "make"
flags = ["-O3", "-Isrc/include"]
EOF
```
Map again and accept the compressor, `t-lzg`, as the program to move; the last line starts `project accept: wrote migration/tools/t-lzg/harness.toml`; `the whole-program check is off` is fine for this page.
```bash
harness project map && harness project accept t-lzg
```

## 3. Move one piece to Rust, writing the answers yourself (about 2 minutes on an M3 Mac)

Every box below starts by going into the copy, so a new window or a break does no harm.
The same steps run in the cockpit (`harness-tui` in `~/lzg-map`), which shows the step you are at on every screen; with Claude Code signed in, its chat makes the test program and the Rust for you (docs/topics/cockpit.md).
Read its C and cut it into pieces, then ask for a test program for the smallest piece, `u-version`; it stops here on purpose: it is waiting for your answer, so the last line is `error: awaiting response: …`.
```bash
cd ~/lzg-map && harness scan --tool t-lzg && harness plan --tool t-lzg && harness gen-driver u-version --tool t-lzg --model guide-written
```
Write the test program into your drafts folder (one box down to `EOF`).
```bash
cat > ~/lzg-practice/version-driver.c <<'EOF'
#include <stdio.h>
#include <string.h>
#include "internal.h"
int main(void)
{
    int i;
    for (i = 0; i < 4; i++) {
        unsigned int num = LZG_Version();
        const char *str = LZG_VersionString();
        printf("call %d LZG_Version=%08x\n", i, num);
        printf("call %d LZG_VersionString=\"%s\" length=%u\n", i, str, (unsigned int)strlen(str));
    }
    return 0;
}
EOF
```
Put it in the answer file the harness is waiting for, in the wrapping it expects, and ask again so it checks your test against the C; look for `-> GREEN`.
````bash
cd ~/lzg-map && REQ=$(ls -t migration/tools/t-lzg/units/u-version/driver-traces/*.request.json | head -n 1) &&
RESP="${REQ%.request.json}.response.json" &&
jq -n --rawfile d ~/lzg-practice/version-driver.c '{text: ("driver.c\n```c\n" + $d + "```\nRUHARNESS_END_OF_OUTPUT\n"), input_tokens: 0, output_tokens: 0, stop_reason: "end_turn"}' > "$RESP" &&
harness gen-driver u-version --tool t-lzg --model guide-written
````
Ask for the Rust; it stops here on purpose: it is waiting for your answer, so the last line is `error: awaiting response: …`.
```bash
cd ~/lzg-map && harness migrate u-version --tool t-lzg --model guide-written
```
Write the Rust (one box down to `EOF`).
````bash
cat > ~/lzg-practice/version-answer.txt <<'EOF'
src/logic.rs
```rust
/// The library's version number, as `LZG_VERNUM`.
pub fn version() -> u32 { 0x0100_000a }
/// The library's version text with its closing zero byte, as `LZG_VERSION`.
pub fn version_string() -> &'static [u8] { b"1.0.10\0" }
```
src/ffi.rs
```rust
#[no_mangle]
pub unsafe extern "C" fn LZG_Version() -> u32 { crate::logic::version() }
#[no_mangle]
pub unsafe extern "C" fn LZG_VersionString() -> *const u8 { crate::logic::version_string().as_ptr() }
```
RUHARNESS_END_OF_OUTPUT
EOF
````
Put it in the answer file and ask again; look for `u-version promoted and verified — status set to verified`. On the command line a green Rust is accepted for you ("promoted" means accepted); the cockpit asks you first.
```bash
cd ~/lzg-map && REQ=$(ls -t migration/tools/t-lzg/units/u-version/traces/*.request.json | head -n 1) &&
RESP="${REQ%.request.json}.response.json" &&
jq -Rs '{text: ., input_tokens: 0, output_tokens: 0, stop_reason: "end_turn"}' ~/lzg-practice/version-answer.txt > "$RESP" &&
harness migrate u-version --tool t-lzg --model guide-written
```
Run the judge yourself; the last line is `verify: u-version GREEN — status set to verified` (the one `[SKIP]` line is expected).
```bash
cd ~/lzg-map && harness verify u-version --tool t-lzg
```

## 4. Break it on purpose and watch the judge catch it (about 1 minute on an M3 Mac)

This changes one digit of the version number the Rust returns; the judge then runs and ends `u-version RED`, with a `[FAIL] differential-driver` line.
```bash
cd ~/lzg-map && sed -i '' 's/0x0100_000a/0x0100_000b/' migration/tools/t-lzg/units/u-version/u_version_rs/src/logic.rs && harness verify u-version --tool t-lzg
```
This puts it back; the judge ends `u-version GREEN — status set to verified` again.
```bash
cd ~/lzg-map && sed -i '' 's/0x0100_000b/0x0100_000a/' migration/tools/t-lzg/units/u-version/u_version_rs/src/logic.rs && harness verify u-version --tool t-lzg
```

## What just happened

The harness read liblzg's C and cut it into pieces.
You gave it a test program and a Rust version of one piece by hand, which a model would normally write.
The judge ran the C and your Rust through that test, compared what they print, and caught the digit you changed.

If a screen does not match, the full walk with every screen and what to do is docs/TESTING-GUIDE.md (Part 0 for setup, Part 12 for this path, Troubleshooting at the end);
the ideas are in docs/TUTORIAL.md;
the deeper pages are in docs/topics/.
