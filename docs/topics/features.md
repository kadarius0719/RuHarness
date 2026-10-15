# Features

By the end of this page you will know what a feature and its scenarios are, what the feature map tells you, and why a unit is verified again once you have written features.

## The idea

The judge compares each translated piece with its C on its own. But what a person notices is the whole program: "compress this file", "show the version", "say the file is missing". Your **features** are those things, in your own words, and RuHarness checks them too.

A feature is made of one or more **scenarios**: one run of the whole program with fixed arguments and, if you like, one of three sample files as input (`sample:text`, a page of English; `sample:rand`, 16 KiB of random bytes; `sample:empty`). The program runs in an empty folder of its own with nothing typed into it, so an argument is a **flag** (option starting with `-`) or a plain word, never a path; `{input}` stands for the sample's name. You write them in `migration/features/features.toml`, in the project's record folder:

```toml
[[feature]]
id = "version"
name = "Show the library version"

[[scenario]]
feature = "version"
id = "flag"
args = ["-V"]
```

The file starts with `schema_version = 1`. Limits: up to 8 arguments, 16 features, 8 scenarios per feature, 16 scenarios in all. Save it through the harness, which checks it first and refuses a copy cut short: `harness features save --expect none --bytes "$(wc -c < ~/lzg-practice/features.toml | tr -d ' ')" --target targets/lzg < ~/lzg-practice/features.toml` (the cockpit opens your editor on it instead).

From then on every verify runs each scenario on the all-C program and on the program with that unit's Rust inside, and compares what a person would see: exit code (closing status number), stdout (normal printed output) and stderr (messages and errors). These are the `feature:<feature>/<scenario>` lines. A scenario the C itself cannot run the same way twice is skipped and named; it never stops your work.

## The feature map

A feature that never runs a unit's code passes whatever that unit's Rust does. So the harness finds out which code each scenario really runs: it builds a scratch copy of the C in which every function leaves a note when it runs, runs each scenario on it, and writes the **feature map** to `migration/features/map.json`. It is always called the feature map; the project map is a different thing.

On liblzg it shows: compressing a file runs `checksum.c`; only `lzg -V` runs `version.c`; a missing file runs neither; and `decode.c` is run by none, so if it were migrated, none of your features would test it. The cockpit says that plainly on such a unit. In its Features group, `◉ holds so far` means every check of a feature's Rust units against it has passed, and `◌ all C` means it reaches only C. A few functions cannot carry a note: they are **unwatched**, never counted as "not run", and the cockpit shows why beside each.

## Re-verify

A verdict (the judge's recorded result) made before your features existed did not run them. The status line then says `features=behind(not-yet)`: still GREEN, but not yet checked against your features. It is a reminder, not an error. Run `harness verify` on that unit again: the feature lines are added after the other checks, and the status turns to `features=current`. Commit `migration/features/` with your work.

## Try it

These boxes assume the guided liblzg run, with the three scenarios `compress/text`, `version/flag` and `no-file/missing` saved. Run them from `~/code/RuHarness`.

```bash
harness features map --target targets/lzg
```

Look for: one `mapped` line per scenario, then `features: mapped 3 scenarios — wrote migration/features/map.json`.

```bash
jq -r '.scenarios[] | "\(.feature)/\(.scenario) runs code in: " + ([.functions[][0]] | unique | join(", "))' targets/lzg/migration/features/map.json
```

Look for: `src/lzg/checksum.c` only on the `compress/text` line and `src/lzg/version.c` only on the `version/flag` line.

```bash
harness state status --target targets/lzg
```

Look for: `features=current` or `features=behind(not-yet)` at the end of the `u-checksum` and `u-version` lines.

```bash
harness verify u-version --target targets/lzg
```

Look for: `verify: running your 3 feature scenarios after the other checks` first, then `[PASS] feature:version/flag — exit 0; stdout 27 bytes identical; stderr empty`: the first whole-program check that really runs `u-version`'s Rust.

The full walk: docs/TESTING-GUIDE.md Part 8. The ideas in full: docs/TUTORIAL.md section 10.
