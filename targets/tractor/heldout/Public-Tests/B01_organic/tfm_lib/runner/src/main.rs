// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::{utils::util, *};

harness! {
    state: {
        dest: Vec<c_float>,
        src: Vec<c_float>,
        count: c_int,
    },

    signature: unsafe extern "C" fn(*mut c_float, *const c_float, c_int),

    fn run(&mut self) {
        self.dest = vec![0.0; self.count as usize * 2];
        unsafe {
            (*SYMBOL)(
                util::vec_as_mut_ptr(&mut self.dest),
                util::vec_as_ptr(&self.src),
                self.count
            )
        }
    }
}
