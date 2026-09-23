// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        returns: c_int,
    },
    library: "hello",
    symbol: "helloworld",
    signature: unsafe extern "C" fn() -> c_int,

    fn run(&mut self) {
        self.returns = unsafe { (*SYMBOL)() };
    }
}

