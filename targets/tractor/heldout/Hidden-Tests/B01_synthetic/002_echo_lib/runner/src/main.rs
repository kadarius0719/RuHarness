// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

harness! {
    state: {
        argc: c_int,
        argv: Vec<CString>,
        returns: c_int
    },
    library: "echo",
    symbol: "echo",

    signature: unsafe extern "C" fn(c_int, *const *const c_char) -> c_int,

    fn run(&mut self) {
        let vec_ptr: Vec<*const c_char> = self.argv
            .iter()
            .map(|s| s.as_ptr())
            .collect();

        self.returns = unsafe {
            (*SYMBOL)(
                self.argc,
                vec_ptr.as_ptr()
            )
        };
    }

}
