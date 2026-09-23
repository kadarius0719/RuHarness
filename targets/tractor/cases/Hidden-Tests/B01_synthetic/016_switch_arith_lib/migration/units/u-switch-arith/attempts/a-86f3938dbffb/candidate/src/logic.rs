use crate::ffi::put_byte;

const RAND_DEG: usize = 31;
const RAND_SEP: usize = 3;

struct GlibcRandom {
    table: [i32; RAND_DEG],
    fptr: usize,
    rptr: usize,
}

impl GlibcRandom {
    fn new(seed: u32) -> Self {
        // glibc's __srandom_r stores `seed == 0 ? 1 : seed` into state[0],
        // but the value that drives the Park-Miller fill for the rest of
        // the table is the raw (unguarded) seed. For every non-zero seed
        // this is the same value either way, so it changes nothing there;
        // it only means seed 0 does not produce the exact same internal
        // state (and therefore the same rand() outputs) as seed 1.
        let raw_seed: i32 = seed as i32;
        let mut table = [0i32; RAND_DEG];
        table[0] = if seed == 0 { 1 } else { raw_seed };
        let mut word: i32 = raw_seed;
        for i in 1..RAND_DEG {
            let hi: i64 = (word as i64) / 127773;
            let lo: i64 = (word as i64) % 127773;
            let mut w: i64 = (16807i64)
                .wrapping_mul(lo)
                .wrapping_sub((2836i64).wrapping_mul(hi));
            if w < 0 {
                w = w.wrapping_add(2147483647);
            }
            word = w as i32;
            table[i] = word;
        }
        let mut rng = GlibcRandom {
            table,
            fptr: RAND_SEP,
            rptr: 0,
        };
        // glibc's __srandom_r discards `rand_deg * 10` outputs while
        // warming up the generator (kc = rand_deg * 10; while (--kc >= 0)
        // random_r(...); — a pre-decrement-and-test loop starting at
        // rand_deg * 10 runs exactly rand_deg * 10 times).
        for _ in 0..(RAND_DEG * 10) {
            rng.next_u32();
        }
        rng
    }

    fn next_u32(&mut self) -> u32 {
        let f = self.table[self.fptr] as u32;
        let r = self.table[self.rptr] as u32;
        let val = f.wrapping_add(r);
        self.table[self.fptr] = val as i32;
        let result = (val >> 1) & 0x7fff_ffff;

        self.fptr += 1;
        if self.fptr >= RAND_DEG {
            self.fptr = 0;
        }
        self.rptr += 1;
        if self.rptr >= RAND_DEG {
            self.rptr = 0;
        }

        result
    }

    fn rand(&mut self) -> u32 {
        self.next_u32()
    }
}

pub fn perform_operations(a: u32, b: u32) -> u32 {
    let safe_b: u32 = if b == 0 { 1 } else { b };
    let shift: u32 = b % 32;

    let add = a.wrapping_add(b);
    let sub = a.wrapping_sub(b);
    let mul = a.wrapping_mul(b);
    let div = a / safe_b;
    let shl = a.wrapping_shl(shift);
    let shr = a.wrapping_shr(shift);
    let xor_v = a ^ b;
    let and_v = a & b;
    let or_v = a | b;
    let not_a = !a;

    add ^ sub ^ mul ^ div ^ shl ^ shr ^ xor_v ^ and_v ^ or_v ^ not_a
}

pub fn switch_arith(seed: u32) {
    let mut rng = GlibcRandom::new(seed);
    let a: u32 = rng.rand();
    let b: u32 = rng.rand();

    let result = perform_operations(a, b);
    let choice = result % 10;

    let message: String = match choice {
        0 => format!(
            "Result: {} — The number is as calm as a sleeping sloth.\n",
            result
        ),
        1 => format!(
            "Result: {} — It tried to divide by zero but thought better of it.\n",
            result
        ),
        2 => format!("Result: {} — Secretly wishes it was a float.\n", result),
        3 => format!(
            "Result: {} — Built entirely from left shifts and dreams.\n",
            result
        ),
        4 => format!(
            "Result: {} — Bitwise ANDed its way into your heart.\n",
            result
        ),
        5 => format!("Result: {} — XOR marks the spot.\n", result),
        6 => format!(
            "Result: {} — Practically a math meme at this point.\n",
            result
        ),
        7 => format!(
            "Result: {} — Stronger than a C macro on Monday morning.\n",
            result
        ),
        8 => format!(
            "Result: {} — Thinks it's the boss of all unsigned ints.\n",
            result
        ),
        9 => format!(
            "Result: {} — May contain traces of peanuts and logic gates.\n",
            result
        ),
        _ => format!("Result: {} — How did we even get here?\n", result),
    };

    for byte in message.as_bytes() {
        put_byte(*byte);
    }
}
