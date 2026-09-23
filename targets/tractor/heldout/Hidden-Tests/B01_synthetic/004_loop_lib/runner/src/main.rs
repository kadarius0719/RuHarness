// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        max_val: c_int,
    },
    library: "Loop",
    symbol: "loop",

    signature: unsafe extern "C" fn(c_int),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.max_val
            )
        };
    }

}
