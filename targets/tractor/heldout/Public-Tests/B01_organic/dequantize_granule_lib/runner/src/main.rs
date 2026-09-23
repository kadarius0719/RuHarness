// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::{utils::util, *};

#[repr(C)]
struct bs_t {
    buf: *const u8,
    pos: c_int,
    limit: c_int,
}

state_member! {
    struct L12_scale_info {
        scf: [[f32; 32]; 6],
        total_bands: u8,
        stereo_bands: u8,
        bitalloc: [[u8; 32]; 2],
        scfcod: [[u8; 32]; 2]
    }
}

harness! {
    state: {
        grbuf: Vec<c_float>,
        b_buf: Vec<u8>,
        b_pos: c_int,
        b_limit: c_int,
        sci: L12_scale_info,
        group_size: c_int,
        returns: c_int,
    },

    signature: unsafe extern "C" fn(*mut c_float, *mut bs_t, *mut L12_scale_info, c_int) -> c_int,

    fn run(&mut self) {
        let mut b = bs_t {
            buf: util::vec_as_mut_ptr(&mut self.b_buf),
            pos: self.b_pos,
            limit: self.b_limit,
        };
        self.returns = unsafe {
            (*SYMBOL)(
                util::vec_as_mut_ptr(&mut self.grbuf),
                &raw mut b as *mut bs_t,
                &raw mut self.sci as *mut L12_scale_info,
                self.group_size,
            )
        };
        self.b_pos = b.pos;
        self.b_limit = b.limit;
    }
}
