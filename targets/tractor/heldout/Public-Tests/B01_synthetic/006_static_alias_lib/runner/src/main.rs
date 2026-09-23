// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        initial_value: c_int,
        iterations: c_int,
    },
    library: "StaticAlias",
    symbol: "driver",
    signature: unsafe extern "C" fn(c_int, c_int),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.initial_value,
                self.iterations
            )
        };
    }
}

