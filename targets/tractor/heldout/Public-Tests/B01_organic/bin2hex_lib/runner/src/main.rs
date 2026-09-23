// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::{utils::util, *};

harness! {
    state: {
        hex: Vec<c_char>,
        hex_maxlen: usize,
        bin: Vec<u8>,
        bin_len: usize,
    },

    signature: unsafe extern "C" fn(*mut c_char, usize, *const u8, usize) -> *mut c_char,

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                util::vec_as_mut_ptr(&mut self.hex),
                self.hex_maxlen,
                util::vec_as_ptr(&self.bin),
                self.bin_len
            )
        };
    }
}
