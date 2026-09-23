// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        base: c_double,
        exponent: c_double,
        returns: c_double
    },
    library: "pow",
    symbol: "feel_the_power",

    signature: unsafe extern "C" fn(c_double, c_double) -> c_double,

    fn run(&mut self) {
        self.returns = unsafe {
            (*SYMBOL)(
                self.base,
                self.exponent           
            )
        };
    }

}
