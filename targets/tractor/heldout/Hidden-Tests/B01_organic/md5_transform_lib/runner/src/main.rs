// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

type tflac_u8 = u8;
type tflac_u32 = u32;
type tflac_u64 = u64;

struct TflacMd5 {
    a: tflac_u32,
    b: tflac_u32,
    c: tflac_u32,
    d: tflac_u32,
    #[expect(dead_code)]
    pos: tflac_u32,
    #[expect(dead_code)]
    total: tflac_u64,
    #[expect(dead_code)]
    buffer: *mut [tflac_u8; 64 + 8],
}

harness! {
    state: {
        a: tflac_u32,
        b: tflac_u32,
        c: tflac_u32,
        d: tflac_u32,
        buffer: [[tflac_u8; 24]; 3],
    },

    signature: unsafe extern "C" fn(*mut TflacMd5),

    fn run(&mut self) {
        let mut input = TflacMd5 {
            a: self.a,
            b: self.b,
            c: self.c,
            d: self.d,
            pos: 0,
            total: 0,
            buffer: &raw mut self.buffer as *mut [tflac_u8; 72]
        };

        unsafe {
            (*SYMBOL)(
                &raw mut input
            )
        };

        self.a = input.a;
        self.b = input.b;
        self.c = input.c;
        self.d = input.d;
    }
}
