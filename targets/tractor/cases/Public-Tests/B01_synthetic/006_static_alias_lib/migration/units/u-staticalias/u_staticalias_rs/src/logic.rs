use std::sync::atomic::{AtomicI32, Ordering};

/// Mirrors the C `static int inner` inside `static_alias`: genuinely
/// persistent for the process lifetime and shared by every call to the
/// exported `static_alias` symbol and by `driver`'s own loop, exactly
/// like the C `static` variable. `AtomicI32` gives interior mutability
/// through plain safe methods (`load`/`store`), which keeps this file
/// free of any manual memory-safety opt-out.
static INNER: AtomicI32 = AtomicI32::new(1);

/// The value-level heart of `int *static_alias(int *outer)`.
///
/// This deliberately never touches a real pointer: `src/ffi.rs` owns
/// the only two raw-memory effects the C function has — reading and
/// possibly writing `*outer` — so this function takes and returns
/// plain `i32`s instead. That also means this module never forms a
/// mutable reference over memory that might actually be `INNER`'s own
/// storage (a caller may legally feed back a pointer this very
/// function returned earlier), which would otherwise race against
/// reading/writing `INNER` through its atomic API in the same call.
///
/// Returns `Some(new_outer_value)` when the C code takes the `else`
/// branch (updates `*outer` and returns `outer`); returns `None` when
/// the C code takes the `if` branch (updates the static `inner` and
/// returns `&inner` — see `inner_ptr`).
pub fn static_alias_step(outer_val: i32) -> Option<i32> {
    let inner_val = INNER.load(Ordering::Relaxed);
    if outer_val >= inner_val {
        let new_inner = inner_val.wrapping_add(outer_val);
        INNER.store(new_inner, Ordering::Relaxed);
        None
    } else {
        Some(outer_val.wrapping_add(inner_val))
    }
}

/// The address of the persistent `INNER` storage, formed only through
/// safe pointer casts (never dereferenced here) so `src/ffi.rs` can
/// hand it back to the C caller as the function's `int*` result.
pub fn inner_ptr() -> *mut i32 {
    (&INNER) as *const AtomicI32 as *mut i32
}

/// Direct translation of `void driver(int initial_value, int iterations)`.
///
/// Tracks, purely by value, whether `running_sum` currently refers to
/// this call's own local copy of the running value or to the shared
/// `INNER` storage, and drives `static_alias_step` exactly like the C
/// loop drives `static_alias`.
pub fn driver(initial_value: i32, iterations: i32) {
    let mut local_value = initial_value;
    let mut aliased = false;
    for _ in 0..iterations {
        let outer_val = if aliased {
            INNER.load(Ordering::Relaxed)
        } else {
            local_value
        };
        match static_alias_step(outer_val) {
            Some(new_val) => {
                local_value = new_val;
                aliased = false;
                print_i32_line(local_value);
            }
            None => {
                aliased = true;
                print_i32_line(INNER.load(Ordering::Relaxed));
            }
        }
    }
}

/// Writes `value` formatted exactly like C's `"%d\n"`, one byte at a
/// time, through the real C `putchar` (via `crate::ffi::put_byte`) so
/// it lands in the same stdio stream the differential driver reads.
fn print_i32_line(value: i32) {
    let text = value.to_string();
    for b in text.as_bytes() {
        crate::ffi::put_byte(*b);
    }
    crate::ffi::put_byte(b'\n');
}
