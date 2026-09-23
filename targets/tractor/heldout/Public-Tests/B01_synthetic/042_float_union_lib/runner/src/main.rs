// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        f: f64,
    },
    library: "driver",
    symbol: "driver",
    signature: unsafe extern "C" fn(f64),

    fn run(&mut self) {
        unsafe { (*SYMBOL)(self.f) };
    }
}

