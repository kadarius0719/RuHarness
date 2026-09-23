// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

type tflac_u8 = u8;
type tflac_s32 = i32;
type tflac_u32 = u32;
type tflac_u64 = u64;

state_member! {
    struct Tflac {
        bitdepth: tflac_u32,
        cur_blocksize: tflac_u32,
        subframe_bitdepth: tflac_u32,
        constant: tflac_u8,
        residual_errors: [tflac_u64; 5usize],
        residuals: [tflac_s32; 5usize],
    }
}

harness! {
    state: {
        tflac: Tflac,
        channel: tflac_u32,
        stride: tflac_u32,
    },

    signature: unsafe extern "C" fn(*mut Tflac, tflac_u32, tflac_u32),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                &raw mut self.tflac as *mut Tflac,
                self.channel,
                self.stride,
            )
        };
    }
}
