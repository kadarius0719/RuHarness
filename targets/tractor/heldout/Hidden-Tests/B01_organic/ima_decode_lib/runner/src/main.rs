// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

type ima_u8_t = c_uchar;
type ima_u16_t = c_ushort;
type ima_u64_t = c_ulonglong;
type ima_s32_t = c_int;
type ima_f32_t = f32;
type ima_output_t = ima_f32_t;

state_member! {
    struct ImaChannelState {
        index: ima_s32_t,
        predict: ima_s32_t,
    }
}

state_member! {
    struct ImaBlock {
        preamble: ima_u16_t,
        data: [ima_u8_t; 32usize],
    }
}

harness! {
    state: {
        output: ima_output_t,
        channel_count: c_uint,
        block: ImaBlock,
        decode_count: ima_u64_t,
        state: ImaChannelState,
    },

    signature: unsafe extern "C" fn(*mut ima_output_t, c_uint, *const ImaBlock, ima_u64_t, *mut ImaChannelState),

    fn run(&mut self) {
        unsafe {
            (*SYMBOL)(
                &raw mut self.output as *mut ima_output_t,
                self.channel_count,
                &raw const self.block as *const ImaBlock,
                self.decode_count,
                &raw mut self.state as *mut ImaChannelState
            )
        };
    }
}
