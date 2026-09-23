// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        base: f64,
        exponent: f64,
        returns: f64,
    },
    library: "pow",
    symbol: "my_pow",
    signature: unsafe extern "C" fn(f64, f64) -> f64,

    fn run(&mut self) {
        self.returns = unsafe {
            (*SYMBOL)(
                self.base,
                self.exponent
            )
        };
    }
}

