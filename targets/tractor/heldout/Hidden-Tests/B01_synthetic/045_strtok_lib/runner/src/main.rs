// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        s_in: CString
    },
    library: "driver",
    symbol: "driver",

    signature: unsafe extern "C" fn(*const c_char),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.s_in.as_ptr()
            )
        };
    }

}
