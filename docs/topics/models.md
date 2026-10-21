# Models

By the end of this page you will know how RuHarness asks a model through the hand-off and its envelope, why you name who answers, how the cockpit's chat uses your Claude subscription, and where a local model and replay fit.

## The hand-off

RuHarness includes no model of its own. It asks one, whichever you choose, and the judge checks every answer the same way, so models are interchangeable. The way it reaches a model is its **provider**. The default is the **hand-off** (provider name `external`): the harness writes its question into a file and stops, and goes on when an answer file appears beside it.

1. A command that needs a model, such as `harness migrate u-checksum --target targets/lzg --model my-claude-code --no-promote`, writes `<key>.request.json` and stops. Its last line starts `error: awaiting response:` and names the answer file, and it ends with exit code (closing status number) 1. That is waiting, not breaking.
2. Someone answers: the cockpit's chat, Claude Code in another window, or you, writing `<key>.response.json` beside the question.
3. The same command again reads the answer and goes on. A migration asks one question per turn (one translation turn, then up to three repair turns), so it may stop several times.

The **key** (question's eight-character name), such as `aaa26e46`, is computed from the question itself: asking the same question again gives the same key, which is how running the command again finds the answer.

## The envelope

An answer is not plain text. The reply must sit in a small **envelope** (the answer's wrapper): `{"text": …, "input_tokens": 0, "output_tokens": 0, "stop_reason": "end_turn"}`. The reply goes in `"text"`; the other three fields are bookkeeping a paid service fills in, and for an answer written by hand they are `0`, `0` and `"end_turn"`. A bare answer is refused with "the response file must hold the envelope …". The harness's own waiting message spells the envelope out, and one line writes it for a reply saved in a file: `jq -Rs '{text: ., input_tokens: 0, output_tokens: 0, stop_reason: "end_turn"}' ~/lzg-practice/handoff/1.answer.txt > "$RESP"`, where `RESP` holds the answer file's name. The reply itself must follow the layout the question asks for: each file's name, its code between lines of three backticks, and the end line `RUHARNESS_END_OF_OUTPUT`.

## Naming who answers

`--model` names who answers, and the name is recorded with every **attempt** (one translation try): `guide-written`, `my-claude-code`, `by-hand`, or a real model's name. Left out, the answer is recorded as `claude-sonnet-5`'s, which would be untrue if someone else answered. Name it on the first run: the name is part of the question, so changing it later asks a new question with a new key, and the harness waits again.

## The cockpit's chat, on your subscription

The easiest way is the cockpit's chat: your own Claude Code, signed in with your Claude subscription, answers the hand-offs a migration or a unit's test program asks, with no **API key** (secret billing code) at all. The cockpit starts it with the line `starting Claude Code (signed in with your Claude subscription)`. Its review dialog names the provider and the model, `A model call: provider external, model <model> (the chat's — it answers the hand-offs)`, and the run records `--requester=chat`. Before you start, check that nothing in your Terminal would make Claude Code bill a key instead of your subscription. If you ever use a key, it lives in your own settings file, never in the project, so a project you open cannot send it anywhere.

In the cockpit, every model item goes through the chat. On a unit, **Make the test program — ask in chat** and **Migrate — ask in chat** type the request into the chat, which asks the cockpit for `harness gen-driver` or `harness migrate` and answers their hand-offs itself. On the Project view, **Ask for advice — ask in chat** and **Ask for a proposal — ask in chat** type the question into the chat (`Which file should t-unlzg keep for choice d1, and why?`, `Propose the build configuration for this project from its Makefiles`); the chat reads the map's open question exactly as `harness project ask` would send it (the harness tool `harness_map_questions`), and its reply is checked by the same rules as a hand-off answer before it is recorded as the chat model's advice (`harness_map_answer`), in `migration/map/project-map.reply.json` (or the proposal in `migration/map/config.proposed.toml`). No answer file to write by hand: `harness project ask` and its hand-off stay the command line's way.

The chat can also **propose** your features and workloads. On the Features row, **Propose features — ask in chat** (on the Speed row, **Propose a workload — ask in chat**) types the question into the chat; the chat reads the program through the harness and writes its proposal with the harness tool `harness_features_propose` (or `harness_workloads_propose`), which checks it by the same rules your save uses and writes only `features.proposed.toml` (or `workloads.proposed.toml`) beside your file, with a header naming the chat's model and the date. The proposal is the model's; the file stays yours: the cockpit opens the proposal in your editor as your draft, and only your save writes it.

If Claude Code is not installed or not signed in, the cockpit says so before anything runs — a dialog titled `Claude Code is not installed` or `Claude Code is not signed in`, with the steps: 1. Open a new Terminal window. 2. Type claude and press Return. 3. Follow the sign-in it shows. 4. Come back here and choose Check again. The same dialog, with what Claude Code said, appears when the chat stops by itself.

## A local model, and replay

A **local model** runs on your own computer (for example through Ollama): free and private. Small models usually fail, and the judge catches it: a wrong answer is RED like any other. Downloading a model is your choice; the guided run never needs one.

**Replay** re-checks a recorded attempt from its saved answers, calling no model at all. It proves a past result still holds, at zero cost.

Whichever provider answers, the model only ever writes two files for a unit, the translated logic and the thin layer that connects it to the C, and everything it writes is built and run inside the **sandbox** (locked-down build space) before the judge sees it.

## Try it

The first two boxes run inside `~/lzg-map`, the guide's copy of the whole liblzg download, after its map and configuration; the others from `~/code/RuHarness`.

```bash
harness project ask --model by-hand
```

Look for: `awaiting response: …/migration/map/traces/<key>.response.json`, the long line spelling out the envelope, and `error: awaiting response: …` last. Once answered, the same box prints the model's advice instead.

```bash
echo "exit=$?"
```

Look for: `exit=1`, run straight after the box above: here 1 means "waiting for an answer".

```bash
env | grep -E '^(ANTHROPIC_|CLAUDE_CODE_USE_)' | cut -d= -f1; echo "check done"
```

Look for: only `check done`; any name above it is a setting that could make Claude Code bill a key instead of your subscription.

```bash
harness state status --target targets/lzg
```

Look for: the `attempts:` line under a unit, such as `[a-<12hex>:external:green]`: each attempt with how it was answered and how it ended.

The full walk: docs/TESTING-GUIDE.md Part 4 (and Part 6). The ideas in full: docs/TUTORIAL.md section 6.
