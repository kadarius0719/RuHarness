// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {},
    library: "driver",
    symbol: "driver",
    signature: unsafe extern "C" fn(),

    fn run(&mut self) {
        unsafe { (*SYMBOL)() };
    }
}

