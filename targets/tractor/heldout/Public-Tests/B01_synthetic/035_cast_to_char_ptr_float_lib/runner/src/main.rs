// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        x: f32,
    },
    library: "driver",
    symbol: "driver",
    signature: unsafe extern "C" fn(f32),

    fn run(&mut self) {
        unsafe { (*SYMBOL)(self.x) };
    }
}

