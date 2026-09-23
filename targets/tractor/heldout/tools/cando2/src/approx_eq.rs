// © 2026 Massachusetts Institute of Technology
// MIT License

use crate::CandoError;
use approx::{Relative, RelativeEq};
use num_traits::FromPrimitive;

/// The difference tolerance for comparing floating point numbers. This should be sufficient but we
/// could think about decreasing this value and enforcing higher precision.
const EPSILON: f64 = 1e-6;

/// Approximate equality for any floating point type (i.e., f32, f64, c_float, c_double). This uses
/// a relative epsilon comparison for approximate equality checking.
///
/// # Returns
///
/// True/false if `a` approximately equals `b`. This function will exit the current process if
/// `EPSILON` can't be converted from a f64 to the `Epsilon` type.
///
/// # Note
///
/// This does NOT currently handle approximate equality for floating point types contained within
/// an `Option` or `Result`
pub fn approx_eq<T: RelativeEq<Epsilon = T> + FromPrimitive>(a: T, b: T) -> bool {
    let epsilon = T::from_f64(EPSILON)
        .ok_or_else(|| CandoError::str_to_err("Couldn't convert f64 to epsilon"))
        .unwrap_or_else(|e| e.exit());
    Relative::default().epsilon(epsilon).eq(&a, &b)
}

#[cfg(test)]
mod test {
    use super::*;
    use libc::{c_double, c_float};

    #[test]
    fn approx_eq_f64() {
        let res = 0.1 + 0.2;
        assert!(approx_eq(res, 0.3));
        assert!(!approx_eq(res, 0.31));
    }

    #[test]
    fn approx_eq_f32() {
        let res: f32 = 0.1 + 0.2;
        assert!(approx_eq(res, 0.3));
        assert!(!approx_eq(res, 0.31));
    }

    #[test]
    fn approx_eq_c_float() {
        let res: c_float = 0.1 + 0.2;
        assert!(approx_eq(res, 0.3));
        assert!(!approx_eq(res, 0.31));
    }

    #[test]
    fn approx_eq_c_double() {
        let res: c_double = 0.1 + 0.2;
        assert!(approx_eq(res, 0.3));
        assert!(!approx_eq(res, 0.31));
    }

    // #[test]
    // fn approx_eq_option_c_double() {
    //     let res: c_double = 0.1 + 0.2;
    //     assert!(approx_eq(Some(res), Some(0.3)));
    //     assert!(!approx_eq(Some(res), Some(0.31)));
    //     assert!(!approx_eq(Some(res), None));
    // }
}
