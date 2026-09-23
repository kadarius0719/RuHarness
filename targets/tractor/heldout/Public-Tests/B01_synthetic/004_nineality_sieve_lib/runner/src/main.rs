// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        start: c_int,
    },
    library: "Sieve",
    symbol: "sieve",
    signature: unsafe extern "C" fn(c_int),

    fn run(&mut self) {
        unsafe { (*SYMBOL)(self.start) };
    }
}

