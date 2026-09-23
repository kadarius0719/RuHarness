// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

state_member! {
    struct Date {
        month: c_int,
        day: c_int,
        year: c_int,
    }
}

harness! {
    state: {
        date: Date,
        returns: bool
    },
    library: "SimpleStruct",
    symbol: "isItMay",

    signature: unsafe extern "C" fn(Date) -> bool,

    fn run(&mut self) {
        self.returns = unsafe {
            (*SYMBOL)(
                self.date.clone()
            )
        };
    }

}
