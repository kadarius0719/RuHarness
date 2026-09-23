// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        c: CString,
    },
    library: "driver",
    symbol: "driver",
    signature: unsafe extern "C" fn(c_char),

    fn run(&mut self) {
        unsafe { (*SYMBOL)(self.c.as_bytes()[0] as c_char) };
    }
}
