// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::{utils::util, *};

type wchar_t = i32;

harness! {
    state: {
        dst: Vec<wchar_t>,
        numElem: usize,
        src: Vec<wchar_t>,
        returns: c_int,
    },

    signature: unsafe extern "C" fn(*mut wchar_t, usize, *const wchar_t) -> c_int,

    fn run(&mut self) {
        self.returns = unsafe {
            (*SYMBOL)(
                util::vec_as_mut_ptr(&mut self.dst),
                self.numElem,
                util::vec_as_ptr(&self.src),
            )
        }
    }
}
