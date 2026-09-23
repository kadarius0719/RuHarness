// © 2026 Massachusetts Institute of Technology
// MIT License

#![cfg_attr(fuzzing, no_main)]

use cando2::*;

state_member! {
    #[derive(Copy)]
    struct lm_vec2 {
        x: c_float,
        y: c_float,
    }
}

harness! {
    state: {
        poly: Vec<lm_vec2>,
        n_poly: c_int,
        clip: Vec<lm_vec2>,
        n_clip: c_int,
        res: Vec<lm_vec2>,
        returns: c_int,
    },

    signature: unsafe extern "C" fn(*mut lm_vec2, c_int, *mut lm_vec2, c_int, *mut lm_vec2) -> c_int,

    fn run(&mut self) {
        let zeroed = lm_vec2 { x: 0.0, y: 0.0 };

        self.n_poly %= 32;
        self.n_clip %= 29;
        self.n_clip += 3;
        self.poly.resize(self.n_poly as usize, zeroed);
        self.clip.resize(self.n_clip as usize, zeroed);

        self.returns = unsafe {
            (*SYMBOL)(
                self.poly.as_mut_ptr(),
                self.n_poly,
                self.clip.as_mut_ptr(),
                self.n_clip,
                self.res.as_mut_ptr()
            )
        }
    }
}
