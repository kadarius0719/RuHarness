// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        block: [[c_uchar; 16]; 4],
        pmax16: c_ushort,
        pmin16: c_ushort,
        mask: c_uint,
        returns: c_int,
    },

    signature: unsafe extern "C" fn(*mut c_uchar, *mut c_ushort, *mut c_ushort, c_uint) -> c_int,

    fn run(&mut self) {
        self.returns = unsafe {
            (*SYMBOL)(
                &raw mut self.block as *mut c_uchar,
                &raw mut self.pmax16,
                &raw mut self.pmin16,
                self.mask,
            )
        };
    }
}
