# Drivers

By the end of this page you will know what a driver is, why the harness tests it before trusting it, and how to answer the driver question yourself.

## The idea

The judge cannot guess how to call a unit (one piece of the program). So each unit has a **driver**: a small C program that calls every function the unit offers, many times, with fixed inputs, and prints every result. The judge builds it once with the unit's C and once with the Rust, and compares what the two print.

A driver has rules, written into the question the harness asks: only `int main(void)` is public; inputs are fixed, with no clocks and no real randomness; it never prints a memory address (those differ between runs); and it never does **undefined behaviour** (anything C leaves open), such as reading through a **null pointer** (address pointing nowhere).

## Why it is tested before it is trusted

A driver that cannot tell broken C from the real thing could not catch a broken translation either, and every GREEN built on it would mean nothing. So what the harness receives is first a **candidate** (kept apart, unused): stored in a folder of its own until it passes seven checks, always in this order:

| Check | What it asks |
|---|---|
| `driver-build` | Does it compile, with strict warnings, and link against the unit's C (and the original C of the pieces it calls)? |
| `driver-shape` | Does it define only `main` and call only the unit and harmless standard functions? Does its text pass the rule check (the lint)? |
| `symbols-called` | Does it call every public function of the unit? |
| `determinism` | Do three runs print exactly the same bytes? |
| `opt-levels` | Does it print the same with the compiler's speed-ups off (`-O0`) and on (`-O2`)? |
| `sanitizers` | Does it run clean under the **sanitizers** (builds that catch misuse)? |
| `mutation` | The harness plants small bugs in copies of the C, called **mutants**. Does the driver print something different for at least 60 % of them ("kill" them)? |

A mutant that compiles to exactly the same machine code as the original is thrown out as "TCE-equivalent": no test could tell it apart. When fewer than 10 mutants count, the driver may miss at most one (the small-n rule). When nothing is left to measure, as in liblzg's tiny `version.c`, the line reads `n/a`, which counts as a pass. A driver that passes all seven is copied into the unit's folder, tested once more there, and recorded in the plan; only then can the unit be migrated.

A unit that calls other pieces (liblzg's `u-encode` calls `u-checksum`) has its driver linked with **the original C of the pieces it calls**, whatever Rust has been accepted for them: a driver is checked against the original program, and the planted bugs go only into the unit's own C. So accepting a callee's Rust later never makes this driver out of date; changing the callee's C does. The judge's own runs (`harness verify`) then link the callees as the program stands (see [the judge](judge.md)).

## Answering the driver question by hand

Normally a model writes the driver. The question goes through a **hand-off** (question left in a file): `harness gen-driver u-checksum --target targets/lzg --model guide-written` writes `<key>.request.json` under the unit's `driver-traces/` folder and stops with `error: awaiting response: …` and exit code (closing status number) 1. That is waiting, not breaking. To answer it yourself:

1. Store the question's path: `REQ=$(ls -t targets/lzg/migration/units/u-checksum/driver-traces/*.request.json | head -n 1)`, and the answer's: `RESP="${REQ%.request.json}.response.json"`.
2. Write your driver into a file of your own, such as `~/lzg-practice/checksum-driver.c`.
3. Wrap it. The reply must be a line `driver.c`, a line of three backticks and `c`, the whole file, a line of three backticks, and the end line `RUHARNESS_END_OF_OUTPUT`, all inside the **envelope** (the answer's wrapper). One line does both: ``jq -n --rawfile d ~/lzg-practice/checksum-driver.c '{text: ("driver.c\n```c\n" + $d + "```\nRUHARNESS_END_OF_OUTPUT\n"), input_tokens: 0, output_tokens: 0, stop_reason: "end_turn"}' > "$RESP"``.
4. Run exactly the same `gen-driver` command again, with the same `--model` word: a different word is a different question with a new key. It ends `-> GREEN` and `promoted migration/units/u-checksum/driver.c`.

If it asks again instead, your answer was not usable: the record of that try says why (`format` for the layout, `build` for compiling, `check` or `oracle` for a broken rule or a weak test).

In the cockpit you need none of this: a unit with no test program offers **Make the test program — ask in chat** (cockpit topic). The chat answers the same question; the cockpit runs `harness gen-driver <unit> --requester=chat`, whose questions wait under the unit's `driver-traces/chat/` and whose answers the chat gives through the cockpit. `migrate` on a unit with no test program says so: `error: u-checksum has no test program yet: make one first (`harness gen-driver u-checksum --target targets/lzg`, or Make the test program in the cockpit)` (with your own `--model` repeated when you gave one).

## Try it

These boxes assume the guided liblzg run, with `u-checksum`'s driver validated. Run them from `~/code/RuHarness`.

```bash
jq -r '.checks[] | "\(.name): \(if .passed then "PASS" else "FAIL" end) - \(.detail)"' targets/lzg/migration/units/u-checksum/driver-validation.json
```

Look for: seven lines with `PASS`, in the order of the table, the last such as `mutation: PASS - killed 12/13 compiled …`.

```bash
grep -A 4 '^\[unit.oracle\]' targets/lzg/migration/plan.toml
```

Look for: `kind = "c-abi-differential"` and `driver = "migration/units/u-checksum/driver.c"`: the plan now says how to judge this unit.

```bash
REQ=$(ls -t targets/lzg/migration/units/u-checksum/driver-traces/*.request.json | head -n 1)
```

Look for: nothing printed; `REQ` now names the question the harness asked.

```bash
jq -r .user "$REQ" | grep '^\['
```

Look for: `[UNIT]`, `[ABI CONTRACT]`, `[C SOURCE]`, `[DRIVER CONTRACT]` and `[TASK]`: the parts of the question a model (or you) answers.

```bash
jq -r '.turns[] | "\(.kind) -> \(.result)"' targets/lzg/migration/units/u-checksum/driver-attempts/d-*/attempt.json
```

Look for: `generate -> green`, the one turn of your accepted driver.

The full walk: docs/TESTING-GUIDE.md Part 3. The ideas in full: docs/TUTORIAL.md section 5.
