// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::{utils::util, *};

#[repr(C)]
#[derive(Clone)]
pub struct ListNode {
    pub value: c_int,
    pub next: *mut ListNode,
}

harness! {
    state: {
        values: Vec<c_int>,
        returns: c_int,
    },
    library: "SimpleList",
    symbol: "smallestValue",
    signature: unsafe extern "C" fn(*mut ListNode) -> c_int,

    fn run(&mut self) {
        let mut nodes = vec![
            ListNode {
                value: 0,
                next: std::ptr::null_mut()
            };
            self.values.len()
        ];

        for i in 0..self.values.len() {
            nodes[i].value = self.values[i];
            if i != self.values.len()-1 {
                nodes[i].next = &raw mut nodes[i+1];
            }
        }

        self.returns = unsafe {
            (*SYMBOL)(
                util::vec_as_mut_ptr(&mut nodes),
            )
        };
    }
}
