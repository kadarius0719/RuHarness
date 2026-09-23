// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        s1: CString,
        s2: CString
    },
    library: "driver",
    symbol: "driver",
    signature: unsafe extern "C" fn(*const c_char, *const c_char),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.s1.as_ptr(),
                self.s2.as_ptr()
            )
        };
    }
}

