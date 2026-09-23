// © 2026 Massachusetts Institute of Technology
// MIT License

use {
    crate::CatastrophicError,
    serde::Deserialize,
    std::{collections::HashMap, io::Read, path::Path, process::Command},
};

/// Key is the name of the counter (e.g., task-clock), value is the value of the counter
/// where if it's `None` the program didn't run long enough to collect it
type PerfCounters = HashMap<String, Option<f64>>;

const PERF_PCNT_RUNNING_THRESHOLD: f64 = 99.0;
/// The perf events that get passed to -e
const PERF_EVENTS: [&str; 15] = [
    "instructions",
    "cpu-cycles",
    "ref-cycles",
    "branch-instructions",
    "branch-misses",
    "cache-misses",
    "cache-references",
    "context-switches",
    "cpu-migrations",
    "page-faults",
    "duration_time",
    "user_time",
    "system_time",
    "task-clock",
    "cpu-clock",
];

/// Builds a `Command` that will run perf-stat
///
/// # Args
///
/// `out_file`: the path to the file that the perf-stat output should be dumped
///
/// # Returns
///
/// A command that can be appended with an executable and other arguments to
/// run under perf-stat
pub fn get_wrapper_cmd(out_file: &Path) -> Command {
    let mut cmd = Command::new("perf");
    cmd.arg("stat")
        .arg("-j")
        .arg("-o")
        .arg(out_file)
        .arg("-e")
        .arg(&PERF_EVENTS.join(","))
        .arg("--");
    cmd
}

/// Converts raw perf-stat JSON output into a more structured version
///
/// # Args
///
/// `bench_file`: file to read raw perf-stat JSON from
///
/// # Returns
///
/// A HashMap representing the cleaned JSON
pub fn to_json(mut bench_file: impl Read) -> Result<PerfCounters, CatastrophicError> {
    let mut raw_perf_json = String::new();
    bench_file.read_to_string(&mut raw_perf_json)?;

    if raw_perf_json.trim().is_empty() {
        return Err(CatastrophicError::str_to_err(
            "`perf stat` did not produce any output. This usually means perf failed before it could write benchmark results.",
        ));
    }

    parse(&raw_perf_json).map_err(|err| {
        CatastrophicError::str_to_err(&format!("`perf stat` produced malformed output: {err}"))
    })
}

/// A `PerfEvent` corresponds to the fields we care about in the raw perf-stat
/// outputted JSON.
#[derive(Debug, Deserialize)]
struct PerfEvent {
    /// The actual value of the counter (e.g., 100000)
    #[serde(rename = "counter-value")]
    counter_value: String,

    /// The name of the event/counter (e.g., instructions)
    #[serde(rename = "event")]
    event_name: String,

    /// The pct time this counter was running. Anything less than PERF_PCNT_RUNNING_THRESHOLD
    /// won't be recorded because it didn't collect enough information to be useful
    #[serde(rename = "pcnt-running")]
    pcnt_running: f64,
}

/// Parses the raw perf-stat JSON output
fn parse(raw_perf_json: &str) -> Result<PerfCounters, CatastrophicError> {
    let mut counters = PerfCounters::new();
    for line in raw_perf_json.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let event: PerfEvent = serde_json::from_str(line)?;

        if event.counter_value == "<not supported>" {
            return Err(CatastrophicError::str_to_err(&format!(
                "Required perf event `{:?}` is not supported in your environment",
                event
            )));
        }

        // From what I've seen there's two reasons that perf will output `<not counted>`
        //  1. the program ran too fast to collect the counter
        //  2. the user requested too many counters and multiplexing occurred
        // So, we'll say that if the pcnt-running was < than the threshold that's a problem
        // with the user's system, otherwise we'll just ignore that counter
        let value = if event.counter_value == "<not counted>" {
            if event.pcnt_running < PERF_PCNT_RUNNING_THRESHOLD {
                return Err(CatastrophicError::str_to_err(&format!(
                    "Your environment doesn't support the number of perf counters required"
                )));
            }
            None
        } else {
            Some(event.counter_value.parse::<f64>()?)
        };

        counters.insert(event.event_name, value);
    }
    Ok(counters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_counters() {
        let json = r#"
{"counter-value": "973733.000000", "unit": "", "event": "instructions", "event-runtime": 374708, "pcnt-running": 100.00}
{"counter-value": "1256220.000000", "unit": "", "event": "cpu-cycles", "event-runtime": 374708, "pcnt-running": 100.00}
{"counter-value": "6016.000000", "unit": "", "event": "branch-misses", "event-runtime": 374708, "pcnt-running": 100.00}
"#;

        let result = parse(json).unwrap();

        assert_eq!(result.get("instructions"), Some(&Some(973733.0)));
        assert_eq!(result.get("cpu-cycles"), Some(&Some(1256220.0)));
        assert_eq!(result.get("branch-misses"), Some(&Some(6016.0)));
    }

    #[test]
    fn test_parse_not_counted_with_high_pcnt() {
        let json = r#"
{"counter-value": "973733.000000", "unit": "", "event": "instructions", "event-runtime": 374708, "pcnt-running": 100.00}
{"counter-value": "<not counted>", "unit": "", "event": "system_time", "event-runtime": 0, "pcnt-running": 100.00}
"#;

        let result = parse(json).unwrap();

        assert_eq!(result.get("instructions"), Some(&Some(973733.0)));
        assert_eq!(result.get("system_time"), Some(&None));
    }

    #[test]
    fn test_parse_not_counted_with_low_pcnt_fails() {
        let json = r#"
{"counter-value": "973733.000000", "unit": "", "event": "instructions", "event-runtime": 374708, "pcnt-running": 100.00}
{"counter-value": "<not counted>", "unit": "", "event": "cache-misses", "event-runtime": 100000, "pcnt-running": 50.00}
"#;

        let result = parse(json);

        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("doesn't support the number of perf counters"));
    }

    #[test]
    fn test_parse_not_supported_fails() {
        let json = r#"
{"counter-value": "973733.000000", "unit": "", "event": "instructions", "event-runtime": 374708, "pcnt-running": 100.00}
{"counter-value": "<not supported>", "unit": "", "event": "some-event", "event-runtime": 0, "pcnt-running": 0.00}
"#;

        let result = parse(json);

        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("not supported in your environment"));
    }

    #[test]
    fn test_parse_empty_input() {
        let json = "";

        let result = parse(json).unwrap();

        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_to_json_empty_input_fails_with_benchmark_message() {
        let err = to_json("".as_bytes()).unwrap_err();
        assert!(format!("{err}").contains("perf stat` did not produce any output"));
    }

    #[test]
    fn test_parse_whitespace_only() {
        let json = "   \n\n  \t  \n";

        let result = parse(json).unwrap();

        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_parse_mixed_empty_lines() {
        let json = r#"

{"counter-value": "973733.000000", "unit": "", "event": "instructions", "event-runtime": 374708, "pcnt-running": 100.00}

{"counter-value": "1256220.000000", "unit": "", "event": "cpu-cycles", "event-runtime": 374708, "pcnt-running": 100.00}

"#;

        let result = parse(json).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.get("instructions"), Some(&Some(973733.0)));
        assert_eq!(result.get("cpu-cycles"), Some(&Some(1256220.0)));
    }

    #[test]
    fn test_parse_decimal_values() {
        let json = r#"
{"counter-value": "717589.123456", "unit": "", "event": "duration_time", "event-runtime": 1, "pcnt-running": 100.00}
{"counter-value": "374708.987654", "unit": "", "event": "task-clock", "event-runtime": 374708, "pcnt-running": 100.00}
"#;

        let result = parse(json).unwrap();

        assert_eq!(result.get("duration_time"), Some(&Some(717589.123456)));
        assert_eq!(result.get("task-clock"), Some(&Some(374708.987654)));
    }

    #[test]
    fn test_parse_zero_values() {
        let json = r#"
{"counter-value": "0.000000", "unit": "", "event": "context-switches", "event-runtime": 374708, "pcnt-running": 100.00}
{"counter-value": "0.000000", "unit": "", "event": "cpu-migrations", "event-runtime": 374708, "pcnt-running": 100.00}
"#;

        let result = parse(json).unwrap();

        assert_eq!(result.get("context-switches"), Some(&Some(0.0)));
        assert_eq!(result.get("cpu-migrations"), Some(&Some(0.0)));
    }

    #[test]
    fn test_parse_invalid_json_fails() {
        let json = r#"
{"counter-value": "973733.000000", "unit": "", "event": "instructions"
"#;

        let result = parse(json);

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_number_fails() {
        let json = r#"
{"counter-value": "not-a-number", "unit": "", "event": "instructions", "event-runtime": 374708, "pcnt-running": 100.00}
"#;

        let result = parse(json);

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_multiple_not_counted() {
        let json = r#"
{"counter-value": "973733.000000", "unit": "", "event": "instructions", "event-runtime": 374708, "pcnt-running": 100.00}
{"counter-value": "<not counted>", "unit": "", "event": "system_time", "event-runtime": 0, "pcnt-running": 100.00}
{"counter-value": "<not counted>", "unit": "", "event": "user_time", "event-runtime": 0, "pcnt-running": 100.00}
{"counter-value": "1256220.000000", "unit": "", "event": "cpu-cycles", "event-runtime": 374708, "pcnt-running": 100.00}
"#;

        let result = parse(json).unwrap();

        assert_eq!(result.len(), 4);
        assert_eq!(result.get("instructions"), Some(&Some(973733.0)));
        assert_eq!(result.get("system_time"), Some(&None));
        assert_eq!(result.get("user_time"), Some(&None));
        assert_eq!(result.get("cpu-cycles"), Some(&Some(1256220.0)));
    }

    #[test]
    fn test_parse_pcnt_running_at_threshold() {
        let json = format!(
            r#"{{"counter-value": "<not counted>", "unit": "", "event": "test-event", "event-runtime": 100000, "pcnt-running": {}}}"#,
            PERF_PCNT_RUNNING_THRESHOLD
        );

        let result = parse(&json).unwrap();

        // At threshold should be accepted
        assert_eq!(result.get("test-event"), Some(&None));
    }

    #[test]
    fn test_parse_pcnt_running_just_below_threshold() {
        let json = format!(
            r#"{{"counter-value": "<not counted>", "unit": "", "event": "test-event", "event-runtime": 100000, "pcnt-running": {}}}"#,
            PERF_PCNT_RUNNING_THRESHOLD - 0.1
        );

        let result = parse(&json);

        // Just below threshold should fail
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_large_counter_values() {
        let json = r#"
{"counter-value": "999999999999.000000", "unit": "", "event": "instructions", "event-runtime": 374708, "pcnt-running": 100.00}
{"counter-value": "1234567890123456.000000", "unit": "", "event": "cpu-cycles", "event-runtime": 374708, "pcnt-running": 100.00}
"#;

        let result = parse(json).unwrap();

        assert_eq!(result.get("instructions"), Some(&Some(999999999999.0)));
        assert_eq!(result.get("cpu-cycles"), Some(&Some(1234567890123456.0)));
    }

    #[test]
    fn test_parse_real_world_example() {
        let json = r#"
{"counter-value" : "973733.000000", "unit" : "", "event" : "instructions", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "0.775129", "metric-unit" : "insn per cycle"}
{"counter-value" : "1256220.000000", "unit" : "", "event" : "cpu-cycles", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "3.352531", "metric-unit" : "GHz"}
{"counter-value" : "1006416.000000", "unit" : "", "event" : "ref-cycles", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "2.685867", "metric-unit" : "G/sec"}
{"counter-value" : "173880.000000", "unit" : "", "event" : "branch-instructions", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "464.041334", "metric-unit" : "M/sec"}
{"counter-value" : "6016.000000", "unit" : "", "event" : "branch-misses", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "3.459857", "metric-unit" : "of all branches", "metric-threshold" : "good"}
{"counter-value" : "1312.000000", "unit" : "", "event" : "cache-misses", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "9.049524", "metric-unit" : "of all cache refs", "metric-threshold" : "less good"}
{"counter-value" : "14498.000000", "unit" : "", "event" : "cache-references", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "38.691461", "metric-unit" : "M/sec"}
{"counter-value" : "0.000000", "unit" : "", "event" : "context-switches", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "0.000000", "metric-unit" : "/sec"}
{"counter-value" : "0.000000", "unit" : "", "event" : "cpu-migrations", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "0.000000", "metric-unit" : "/sec"}
{"counter-value" : "57.000000", "unit" : "", "event" : "page-faults", "event-runtime" : 374708, "pcnt-running" : 100.00, "metric-value" : "152.118450", "metric-unit" : "K/sec"}
{"counter-value" : "<not counted>", "unit" : "", "event" : "system_time", "event-runtime" : 0, "pcnt-running" : 100.00}
"#;

        let result = parse(json).unwrap();

        assert_eq!(result.len(), 11);
        assert_eq!(result.get("instructions"), Some(&Some(973733.0)));
        assert_eq!(result.get("cpu-cycles"), Some(&Some(1256220.0)));
        assert_eq!(result.get("context-switches"), Some(&Some(0.0)));
        assert_eq!(result.get("system_time"), Some(&None));
    }
}
