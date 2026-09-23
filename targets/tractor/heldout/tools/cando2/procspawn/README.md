# Forked Procspawn

Fork of `procspawn` crate (links to original crate below). The main
functionality that this fork adds is it changes the `join` and `join_timeout` APIs
to pass the `ExitStatus` of the child process back to the caller. This allows
better error handling to capture if the child process terminated because of a
signal (like `SIGSEGV`) or a Rust panic that returns code 101.

[![Build Status](https://github.com/mitsuhiko/procspawn/workflows/Tests/badge.svg?branch=master)](https://github.com/mitsuhiko/procspawn/actions?query=workflow%3ATests)
[![Crates.io](https://img.shields.io/crates/d/procspawn.svg)](https://crates.io/crates/procspawn)
[![Documentation](https://docs.rs/procspawn/badge.svg)](https://docs.rs/procspawn)
[![rustc 1.65.0](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://img.shields.io/badge/rust-1.70%2B-orange.svg)

This crate provides the ability to spawn processes with a function similar
to `thread::spawn`.  Instead of closures it passes [`serde`](https://serde.rs/)
serializable objects.  The return value from the spawned closure also must be
serializable and can then be retrieved from the returned join handle.

If the spawned function causes a panic it will also be serialized across
the process boundaries.

## Example

Step 1: invoke `procspawn::init` at a point early in your program (somewhere at
the beginning of the main function).  Whatever happens before that point also
happens in your spawned functions.

```rust
procspawn::init();
```

Step 2: now you can start spawning functions:

```rust
let data = vec![1, 2, 3, 4];
let handle = procspawn::spawn(data, |data| {
    println!("Received data {:?}", &data);
    data.into_iter().sum::<i64>()
}, None);
let result = handle.join().unwrap();
```

To run the subprocess through a wrapper such as `perf stat` or `valgrind`,
build a `std::process::Command` for that wrapper and pass it as the third
argument. The wrapper should already contain any required separator such as
`--`; `procspawn` appends the program it wants to run after that.

```rust
let mut wrapper = std::process::Command::new("perf");
wrapper.arg("stat").arg("--");

let handle = procspawn::spawn((1, 2), |(a, b)| a + b, Some(wrapper));
assert_eq!(handle.join().unwrap(), 3);
```

## License and Links

- [Documentation](https://docs.rs/procspawn/)
- [Issue Tracker](https://github.com/mitsuhiko/procspawn/issues)
- [Examples](https://github.com/mitsuhiko/procspawn/tree/master/examples)
- License: [Apache-2.0](https://github.com/mitsuhiko/procspawn/blob/master/LICENSE-APACHE)
