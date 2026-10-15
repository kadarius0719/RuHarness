# Speed

By the end of this page you will know what a workload is, how to read the three kinds of answer speed gives, and what it means when a speed row is out of date.

## The idea

The judge checks that the Rust does the same thing as the C. **Speed** (the command `harness perf`, so the screens say **perf**) tells you whether it does it as fast. It is never part of a verdict (the judge's result): a slower unit is still a correct one, and you decide whether the difference matters. It runs on macOS for now.

## Workloads

A **workload** is one run of the whole program the way it is really used: your own options and, if you like, one input file of yours inside the project (a real file, at most 64 MiB, about 64 million bytes). You write them in `migration/perf/workloads.toml`:

```toml
schema_version = 1

[[workload]]
id = "best"
args = ["-9", "{input}"]
input = "bench/big.txt"
```

`{input}` stands for the input file. `runs` (5 to 31) says how many times each side runs; left out, it is 15. Save it through the harness, which checks it first: `harness perf save --expect none --bytes "$(wc -c < ~/lzg-practice/workloads.toml | tr -d ' ')" --target targets/lzg < ~/lzg-practice/workloads.toml`. `harness perf init --target targets/lzg` writes a starter file with these rules as comments. Measuring with no workloads file is refused with `error: write your workloads file first …`.

For each workload perf runs the original C alone, then each verified unit's Rust swapped in on its own, then every verified unit together ("the program as it stands"). The C and the Rust take turns, 15 times each by default, on the same computer. It measures **CPU time** (processor's working time), which varies a little from run to run, and **instructions** (processor's basic steps), which vary much less.

## The three answers

| Kind | Reads like | What to do |
|---|---|---|
| An answer | `about as fast as the C (within 2 %)`, `slower by about 6.2 % (4.1–8.3 %)`, `faster …`, `probably slower …`, `close call …` | Nothing, or decide whether it matters. The range is where the true value lies; perf is at least 95 % sure of it. |
| Can't tell | `can't tell: the estimate is ±Y %` | The runs varied too much: measure again on a quiet computer; the line gives the command for 31 runs. |
| Too short | `too short to time` | The C ran too briefly (perf wants half a second of CPU time, or a billion instructions): use a bigger input. More runs will not help. |

One more line can appear, and it is not about speed: `behaves differently` means the Rust printed or ended differently from the C on that workload. The judge never runs your workloads, so only perf can find this; both outputs are kept in `migration/build/.perf-out/`.

## Out of date

Every row records what it measured: the workload (its options and its input's bytes), the C, each unit's Rust, the way it measured, and the computer. When one of them changes, the row says which, such as `out of date: your workload changed`, and it stays on record until you measure again. The same bytes again make it current. Numbers belong to one computer at one time, and measuring again replaces a row: commit `migration/perf/` to keep a history.

## Try it

These boxes assume the guided liblzg run, with both units verified and the input `targets/lzg/bench/big.txt` already made (the last box makes it). Run them from `~/code/RuHarness`, and keep the computer quiet during the first.

```bash
harness perf run --target targets/lzg; echo "exit=$?"
```

Look for: a line per unit and workload such as `perf: u-checksum on best — about as fast as the C (within 2 %)`, and at the end `perf: measured 8 rows, 0 too short, 0 behave differently …` and `exit=0`.

```bash
harness perf show --target targets/lzg
```

Look for: the same rows again, each with its CPU time, instructions and memory, without measuring anything.

```bash
echo "one more line" >> targets/lzg/bench/big.txt
```

Look for: nothing printed; the input is now different from the one measured.

```bash
harness perf show --target targets/lzg --no-check | head -4
```

Look for: the third line ending `15 runs · out of date: your workload changed`.

```bash
for i in $(seq 1 900); do cat targets/lzg/src/lzg/*.c; done > targets/lzg/bench/big.txt
```

Look for: nothing printed; the input is made again, byte for byte, and `harness perf show --target targets/lzg` no longer says `out of date`.

The full walk: docs/TESTING-GUIDE.md Part 11. The ideas in full: docs/TUTORIAL.md section 12.
