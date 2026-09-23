// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        seed: c_uint
    },
    library: "switch-arith",
    symbol: "switch_arith",

    signature: unsafe extern "C" fn(c_uint),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.seed
            )
        };
    }

}
