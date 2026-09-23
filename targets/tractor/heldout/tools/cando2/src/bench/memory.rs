// © 2026 Massachusetts Institute of Technology
// MIT License

use {
    super::BenchError,
    crate::CatastrophicError,
    serde::Serialize,
    std::{io::Read, process::Command},
};

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct MassifSnapshot {
    /// Bytes allocated to the heap by the program at the current time
    heap_mem: u64,
    /// Extra bytes allocated to the heap at the current time (from things like the memory
    /// allocator)
    heap_mem_extra: u64,
    /// Bytes allocated to the stack at the current time
    stack_mem: u64,
}

impl MassifSnapshot {
    /// Converts `heap_mem`, `heap_mem_extra`, and `stack_mem` into `MassifSnapshot`
    /// and raising and error if any of them are `None` (the snapshot was malformed)
    fn new(
        heap_mem: Option<u64>,
        heap_mem_extra: Option<u64>,
        stack_mem: Option<u64>,
    ) -> Result<Self, CatastrophicError> {
        Ok(Self {
            heap_mem: heap_mem.ok_or_else(|| {
                CatastrophicError::str_to_err(
                    "Invalid format for massif file: mem_heap_B field missing",
                )
            })?,
            heap_mem_extra: heap_mem_extra.ok_or_else(|| {
                CatastrophicError::str_to_err(
                    "Invalid format for massif file: mem_heap_extra_B field missing",
                )
            })?,
            stack_mem: stack_mem.ok_or_else(|| {
                CatastrophicError::str_to_err(
                    "Invalid format for massif file: mem_stacks_B field missing",
                )
            })?,
        })
    }
}

/// Builds a `Command` that will run valgrind massif
///
/// # Args
///
/// `out_file`: the path to the file that valgrind massif's output should be dumped
///
/// # Returns
///
/// A command that can be appended with an executable and other arguments to
/// run under valgrind massif
pub fn get_wrapper_cmd(out_file: &std::path::Path) -> Command {
    let mut cmd = Command::new("valgrind");
    cmd.arg("--tool=massif")
        .arg("--stacks=yes")
        .arg(format!("--massif-out-file={}", out_file.display()))
        .arg("--");
    cmd
}

/// Converts raw valgrind massif output to JSON
///
/// # Args
///
/// `bench_file`: file that valgrind output raw data to
///
/// # Returns
///
/// A Vec of the different snapshots from the valgrind run
pub fn to_json(mut bench_file: impl Read) -> Result<Vec<MassifSnapshot>, BenchError> {
    let mut massif_out = String::new();
    bench_file
        .read_to_string(&mut massif_out)
        .map_err(BenchError::internal)?;

    if massif_out.trim().is_empty() {
        return Err(BenchError::BenchFail(
            "Benchmarking with `--bench memory` failed because `valgrind massif` did not produce any output. The benchmarked program may have exited before benchmark output could be collected.".to_string(),
        ));
    }

    parse(&massif_out).map_err(|err| {
        BenchError::internal_str(&format!(
            "`valgrind massif` produced malformed output: {err}"
        ))
    })
}

/// Actually parses the raw valgrind output, handling any errors or invalid inputs
fn parse(massif_contents: &str) -> Result<Vec<MassifSnapshot>, CatastrophicError> {
    let mut snapshots: Vec<MassifSnapshot> = Vec::new();
    let mut lines = massif_contents.lines().peekable();

    while let Some(line) = lines.next() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.starts_with("snapshot=") {
            continue;
        }

        let mut heap_mem = None;
        let mut heap_mem_extra = None;
        let mut stack_mem = None;

        while let Some(line) = lines.peek().copied() {
            let line = line.trim();
            if line.starts_with("snapshot=") {
                break;
            }

            let line = lines.next().unwrap().trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            match key {
                "mem_heap_B" => heap_mem = Some(value.parse::<u64>()?),
                "mem_heap_extra_B" => heap_mem_extra = Some(value.parse::<u64>()?),
                "mem_stacks_B" => stack_mem = Some(value.parse::<u64>()?),
                _ => {}
            }
        }

        snapshots.push(MassifSnapshot::new(heap_mem, heap_mem_extra, stack_mem)?)
    }

    Ok(snapshots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_input() {
        assert_eq!(parse("").unwrap(), Vec::<MassifSnapshot>::new());
    }

    #[test]
    fn to_json_empty_input_fails_with_benchmark_message() {
        match to_json("".as_bytes()).unwrap_err() {
            BenchError::BenchFail(message) => {
                assert!(message.contains("valgrind massif` did not produce any output"));
            }
            BenchError::Internal(_) => panic!("empty massif output should be a benchmark failure"),
        }
    }

    #[test]
    fn parse_multiple_snapshots() {
        let massif = r#"
desc: --tool=massif --stacks=yes
cmd: ./driver
time_unit: i
#-----------
snapshot=0
#-----------
time=0
mem_heap_B=0
mem_heap_extra_B=0
mem_stacks_B=0
heap_tree=empty
#-----------
snapshot=1
#-----------
time=12345
mem_heap_B=4096
mem_heap_extra_B=128
mem_stacks_B=512
heap_tree=peak
n2: 4096 (heap allocation functions) malloc/new/new[], --alloc-fns, etc.
 n1: 4096 0x123456: main (example.c:42)
"#;

        assert_eq!(
            parse(massif).unwrap(),
            vec![
                MassifSnapshot {
                    heap_mem: 0,
                    heap_mem_extra: 0,
                    stack_mem: 0,
                },
                MassifSnapshot {
                    heap_mem: 4096,
                    heap_mem_extra: 128,
                    stack_mem: 512,
                },
            ]
        );
    }

    #[test]
    fn parse_ignores_preamble_and_tree_lines() {
        let massif = r#"
desc: --tool=massif --stacks=yes
cmd: ./driver
time_unit: i
"#;

        assert_eq!(parse(massif).unwrap(), Vec::<MassifSnapshot>::new());
    }

    #[test]
    fn parse_fails_when_required_field_is_missing() {
        let massif = r#"
snapshot=0
time=0
mem_heap_B=10
mem_stacks_B=3
heap_tree=empty
"#;

        let err = parse(massif).unwrap_err();
        assert!(format!("{err:?}").contains("mem_heap_extra_B field missing"));
    }

    #[test]
    fn parse_fails_on_invalid_numeric_value() {
        let massif = r#"
snapshot=0
time=0
mem_heap_B=ten
mem_heap_extra_B=5
mem_stacks_B=3
"#;

        assert!(parse(massif).is_err());
    }
}
