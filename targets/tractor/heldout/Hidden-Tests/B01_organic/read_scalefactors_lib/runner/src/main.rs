// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

state_member! {
    struct bs_t {
        buf: Vec<u8>,
        pos: c_int,
        limit: c_int,
    }
}

harness! {
    state: {
        bs: Vec<bs_t>,
        pba: Vec<u8>,
        scfcod: Vec<u8>,
        bands: c_int,
        scf: Vec<c_float>
    },

    signature: unsafe extern "C" fn(*mut bs_t, *mut u8, *mut u8, c_int, *mut c_float),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                self.bs.as_mut_ptr(),
                self.pba.as_mut_ptr(),
                self.scfcod.as_mut_ptr(),
                self.bands,
                self.scf.as_mut_ptr(),
            )
        }
    }
}
