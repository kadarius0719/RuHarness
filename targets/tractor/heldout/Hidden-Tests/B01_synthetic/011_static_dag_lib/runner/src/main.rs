// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        val: c_int,
        iterations: c_int
    },
    library: "StaticDag",
    symbol: "driver",

    signature: unsafe extern "C" fn(c_int, c_int),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.val,
                self.iterations            
            )
        };
    }

}
