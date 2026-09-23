// © 2026 Massachusetts Institute of Technology
// MIT License

//! The entrypoint for testing binaries
use cando2::{runners::bin_runner::conduct_bin, CandoError};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let raw_args: Vec<&str> = args.iter().map(|e| &**e).collect();

    match conduct_bin(&raw_args) {
        Ok(res) => {
            if res.any_bench_failed {
                CandoError::AnyBenchFailed.exit()
            }
            if res.any_vector_failed {
                CandoError::AnyVectorFailed.exit()
            }
        }
        Err(e) => e.exit(),
    }
}
