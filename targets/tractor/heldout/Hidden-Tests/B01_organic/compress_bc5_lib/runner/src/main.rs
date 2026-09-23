// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        dest: [[c_uchar; 16]; 4],
        src: [[c_uchar; 16]; 4]
    },

    signature: unsafe extern "C" fn(*mut c_uchar, *const c_uchar),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                &raw mut self.dest as *mut c_uchar,
                &raw const self.src as *const c_uchar
            )
        };
    }
}
