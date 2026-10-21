# The judge

By the end of this page you will know what the judge checks, what GREEN really means and what it does not, and how to break it on purpose to watch it work.

## The idea

The judge (the files call it the oracle) is the part of RuHarness that decides whether a translation is right. It asks nobody's opinion: not yours, not the model's. It builds the unit (one program piece) twice, once with the original C and once with the new Rust, runs both on the same inputs, and demands the same output, byte for byte. Output means what a program prints normally (**stdout**) and its messages and errors (**stderr**). When the two differ, it says where: "first diff at byte 23" means bytes 0 to 22 were the same.

`harness verify` runs the judge on a unit and prints one line per check, always in this order:

| On screen | What it asks |
|---|---|
| `symbol-set` | Does the Rust offer exactly the functions the C unit did, and nothing extra? |
| `capabilities` | Does the Rust use no more of the computer than the C did: no files, network, other programs or clocks? |
| `driver-shape` | Is the driver (the unit's test program) still calling only the unit and harmless standard functions? |
| `rust-build` | Does the Rust compile? It appears only when it does not, and is then the only check. |
| `differential-driver` | Does every output of the driver match, C against Rust, byte for byte? |
| `whole-program:<sample>` | The whole real program, all-C and with the Rust inside, on one sample file: same exit code (closing status number) and output? |
| `sanitizers` | Does the C side of the driver run clean under the memory checkers? It checks the C and the driver, not the Rust. |
| `boundary` | For some units only: given memory sized exactly as the C needs, does the Rust touch nothing more? |
| `feature:<feature>/<scenario>` | Each of your own uses of the program, run both ways: same exit code, stdout and stderr? |

Every check passes: **GREEN**, and the command ends with exit code 0. Anything else: **RED**, exit code 10, and a unit that was verified is **demoted** (lowered to in-progress). `[SKIP] … not run` is not a pass: it means that check was never set up.

**What the verdict remembers.** A **verdict** (the judge's recorded result) lists every check and a **fingerprint** (code from file bytes), written `blake3:` and 64 characters, of exactly the C, the driver and the Rust it tested. Change one byte of any of them and the unit shows as out of date instead of quietly staying GREEN.

**A piece that calls other pieces.** liblzg's `u-encode` calls `u-checksum`'s function. Every build the judge makes for it links the pieces it calls **as the program stands**: their Rust once you have accepted it, else their C. So `u-encode` is tested with the code the program really runs, and its verdict also records each piece it was tested with, in which form (`c` or `rust`), with its fingerprint (`jq '.inputs.dependencies' …/oracle-latest.json` shows them). When `u-checksum`'s accepted Rust changes, `u-encode` reads `out of date (a piece it calls changed)` until a re-check. Changing `checksum.c` once its Rust is accepted does not: the program runs the Rust, not that C. The guide's Part 7 walks it.

## What GREEN means, and what it does not

GREEN means: on every input the judge tried, the Rust did exactly what the C did. Nothing more.

- It does not prove the Rust is right on inputs nobody tried. A weak driver passes whatever the Rust does. That is why a driver is itself tested before it is trusted.
- A whole-program run that never calls the unit proves nothing about it. liblzg's `u-version` shows this: `lzg -9 <file>` compresses and never asks for the version, so its three whole-program checks pass whatever its Rust does. Only its driver, and a run of `lzg -V` written as a feature, really test it.
- `sanitizers` passing says the test itself is clean, not that the Rust is.

So the judge is strong exactly where it looks, and blind elsewhere. Your features add places for it to look, and accepting a translation stays your decision.

## Try it

These boxes assume the guided liblzg run, with `u-checksum` verified and your work committed (`git status --short` prints nothing), so the last box can put everything back. Run them from `~/code/RuHarness`.

```bash
harness verify u-checksum --target targets/lzg
```

Look for: every line `[PASS]`, and the last ending `u-checksum GREEN — status set to verified`.

```bash
cat targets/lzg/migration/units/u-checksum/oracle-latest.md
```

Look for: `Verdict: **GREEN**`, then under `Inputs tested:` the fingerprints of the C, the Rust and the driver.

```bash
nano -w targets/lzg/migration/units/u-checksum/u_checksum_rs/src/logic.rs
```

Look for: the line where `a` is set to `1` (such as `let mut a: u16 = 1;`); change that `1` to `0`, save with Ctrl-O and Return, leave with Ctrl-X. One character, and the Rust is now wrong.

```bash
harness verify u-checksum --target targets/lzg
```

Look for: `[FAIL] differential-driver — outputs differ (lens 2936 vs 2936, first diff at byte 23)`, the whole-program checks with data failing at byte 11 (where the checksum sits in the compressed file, and `feature:compress/text` too if you wrote features), `sample_empty` still passing because it never calls the checksum, and the last line `u-checksum RED — status demoted verified -> in-progress`.

```bash
git checkout targets/lzg
```

Look for: `Updated 4 paths from the index`; run the first box again and it is GREEN once more.

The full walk: docs/TESTING-GUIDE.md Part 5. The ideas in full: docs/TUTORIAL.md section 7.
