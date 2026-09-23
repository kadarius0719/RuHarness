// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

type btac1c_byte = c_uchar;
type btac1c_u16 = c_ushort;
type btac1c_s16 = c_short;

state_member! {
    struct Btac1cIdxstate {
        idx: btac1c_u16,
        lpred: btac1c_s16,
        rpred: btac1c_s16,
        tag: btac1c_byte,
        bcfcn: btac1c_byte,
        bsfcn: btac1c_byte,
        usefx: btac1c_byte,
        firfx: [[btac1c_s16; 4usize]; 8usize],
    }
}

harness! {
    state: {
        psamp: [[c_int; 32]; 32],
        idx: c_int,
        pfcn: c_int,
        ridx: Btac1cIdxstate,
        returns: c_int,
    },

    signature: unsafe extern "C" fn(*mut c_int, c_int, c_int, *mut Btac1cIdxstate) -> c_int,

    fn run(&mut self) {
        self.returns = unsafe {
            (*SYMBOL)(
                self.psamp.as_mut_ptr() as *mut c_int,
                self.idx,
                self.pfcn,
                &raw mut self.ridx as *mut Btac1cIdxstate,
            )
        };
    }
}
