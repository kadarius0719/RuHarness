// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::{utils::util, *};

harness! {
    state: {
        dest: Vec<c_float>,
        size: c_int,
        radius: c_float
    },

    signature: unsafe extern "C" fn(*mut c_float, c_int, c_float),

    fn run(&mut self) {
        self.dest = vec![0.0; self.size as usize];
        unsafe {
            (*SYMBOL)(
                util::vec_as_mut_ptr(&mut self.dest),
                self.size,
                self.radius
            )
        }
    }
}
