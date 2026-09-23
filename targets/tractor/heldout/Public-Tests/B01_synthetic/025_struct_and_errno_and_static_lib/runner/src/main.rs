// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        input: CString,
    },
    library: "driver",
    symbol: "driver",
    signature: unsafe extern "C" fn(*const c_char),

    fn run(&mut self) {
        unsafe { (*SYMBOL)(self.input.as_ptr()) };
    }
}

