// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

type btac1c_s16 = c_short;

harness! {
    state: {
        ibuf0: [btac1c_s16; 32],
        ibuf1: [btac1c_s16; 32],
        len: c_int,
        returns: c_int,
    },

    signature: unsafe extern "C" fn(*mut btac1c_s16, *mut btac1c_s16, c_int) -> c_int,

    fn run(&mut self) {
        self.returns = unsafe {
            (*SYMBOL)(
                &raw mut self.ibuf0 as *mut btac1c_s16,
                &raw mut self.ibuf1 as *mut btac1c_s16,
                self.len
            )
        };
    }
}
