// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

use std::ptr;

harness! {
    state: {
        mystr: CString,
        start_ptr: Option<c_int>,
        stop_ptr: Option<c_int>,
        returns: c_int,
    },
    library: "String_Slice",
    symbol: "slice",
    signature: unsafe extern "C" fn(*mut c_char, *mut c_int, *mut c_int) -> c_int,

    fn run(&mut self) {
        let start_ptr = if let Some(ref mut start_val_r) = self.start_ptr {
            start_val_r
        } else {
            ptr::null_mut()
        };

        let stop_ptr = if let Some(ref mut stop_val_r) = self.stop_ptr {
            stop_val_r
        } else {
            ptr::null_mut()
        };

        self.returns = unsafe {
            (*SYMBOL)(
                self.mystr.as_ptr() as *mut c_char,
                start_ptr,
                stop_ptr
            )
        };
    }
}
