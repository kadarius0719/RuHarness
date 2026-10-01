//! The statistics under perf's words (docs/PERF-DESIGN.md §3.8): the
//! Hodges–Lehmann shift of two samples' log values and its distribution-free
//! interval from the exact Mann–Whitney null. Std only.

/// Fewest values on either side that give an interval at all.
pub const MIN_VALUES: usize = 5;

/// Most runs a side (the workloads file's `runs` and `--runs` cap).
pub const MAX_RUNS: usize = 31;

/// The exact null distribution of the Mann–Whitney U for sample sizes `m`
/// and `n`: `counts[u]`, the number of the C(m+n, m) orderings with U = u,
/// for u in 0..=m·n. The Gaussian binomial coefficient [m+n choose m] in q,
/// built factor by factor: × (1 − q^(n+i)) then ÷ (1 − q^i), i = 1..=m.
pub fn mann_whitney_counts(m: usize, n: usize) -> Vec<u128> {
    let len = m * n + 1;
    let mut c: Vec<i128> = vec![0; len];
    c[0] = 1;
    for i in 1..=m {
        let step = n + i;
        for k in (step..len).rev() {
            c[k] -= c[k - step];
        }
        for k in i..len {
            c[k] += c[k - i];
        }
    }
    c.into_iter()
        .map(|v| u128::try_from(v).unwrap_or(0))
        .collect()
}

/// The largest u with P(U ≤ u) ≤ 0.025 under the exact (m, n) null — the
/// interval is `[D(c+1), D(m·n−c)]` of the sorted pairwise differences — or
/// `None` when even P(U = 0) is above it (too few values).
pub fn critical_value(m: usize, n: usize) -> Option<usize> {
    let counts = mann_whitney_counts(m, n);
    let total: u128 = counts.iter().sum();
    let mut below = 0u128;
    let mut found = None;
    for (u, count) in counts.iter().enumerate() {
        below += count;
        // P(U ≤ u) ≤ 1/40, exactly.
        if below * 40 <= total {
            found = Some(u);
        } else {
            break;
        }
    }
    found
}

/// The interval's confidence at (m, n): 1 − 2·P(U ≤ c), or `None` with no
/// critical value.
pub fn confidence(m: usize, n: usize) -> Option<f64> {
    let c = critical_value(m, n)?;
    let counts = mann_whitney_counts(m, n);
    let total: u128 = counts.iter().sum();
    let below: u128 = counts[..=c].iter().sum();
    Some(1.0 - 2.0 * below as f64 / total as f64)
}

/// A Hodges–Lehmann shift in log space: `other` against `base`, the median
/// of every `ln(other_i / base_j)` and its interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shift {
    /// The median pairwise log-ratio.
    pub estimate: f64,
    /// The interval's lower end, `D(c+1)`.
    pub lo: f64,
    /// The interval's upper end, `D(m·n−c)`.
    pub hi: f64,
    /// Values used on the base side.
    pub m: usize,
    /// Values used on the other side.
    pub n: usize,
}

impl Shift {
    /// The estimate as a percent change: (e^d − 1) × 100.
    pub fn percent(&self) -> f64 {
        percent(self.estimate)
    }

    /// The interval as percent changes.
    pub fn percent_interval(&self) -> (f64, f64) {
        (percent(self.lo), percent(self.hi))
    }
}

/// A log-ratio as a percent change.
pub fn percent(d: f64) -> f64 {
    d.exp_m1() * 100.0
}

/// A value that can stand in a sample: present, finite and above zero.
pub fn usable(v: Option<f64>) -> Option<f64> {
    v.filter(|x| x.is_finite() && *x > 0.0)
}

/// The shift of `other` against `base` over the values that can stand
/// ([`usable`]; a missing one drops its run whole before any pair is
/// formed), or `None` with fewer than [`MIN_VALUES`] on either side.
pub fn hodges_lehmann(base: &[Option<f64>], other: &[Option<f64>]) -> Option<Shift> {
    let b: Vec<f64> = base
        .iter()
        .filter_map(|v| usable(*v))
        .map(f64::ln)
        .collect();
    let o: Vec<f64> = other
        .iter()
        .filter_map(|v| usable(*v))
        .map(f64::ln)
        .collect();
    if b.len() < MIN_VALUES || o.len() < MIN_VALUES {
        return None;
    }
    let (m, n) = (b.len(), o.len());
    let c = critical_value(m, n)?;
    let mut d: Vec<f64> = Vec::with_capacity(m * n);
    for oi in &o {
        for bj in &b {
            d.push(oi - bj);
        }
    }
    d.sort_by(f64::total_cmp);
    Some(Shift {
        estimate: median_sorted(&d),
        lo: d[c],
        hi: d[m * n - c - 1],
        m,
        n,
    })
}

/// The median of a sorted, non-empty slice (the mean of the middle two at
/// an even length).
pub fn median_sorted(sorted: &[f64]) -> f64 {
    let k = sorted.len();
    if k % 2 == 1 {
        sorted[k / 2]
    } else {
        (sorted[k / 2 - 1] + sorted[k / 2]) / 2.0
    }
}

/// The median of the usable values, or `None` when there are none.
pub fn median(values: &[Option<f64>]) -> Option<f64> {
    let mut v: Vec<f64> = values.iter().filter_map(|x| usable(*x)).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(median_sorted(&v))
}

/// 1.4826 × the median absolute deviation of `values` (finite ones only),
/// or `None` with fewer than two.
pub fn robust_spread(values: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.len() < 2 {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let mid = median_sorted(&v);
    let mut dev: Vec<f64> = v.iter().map(|x| (x - mid).abs()).collect();
    dev.sort_by(f64::total_cmp);
    Some(1.4826 * median_sorted(&dev))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binomial(n: u128, k: u128) -> u128 {
        (1..=k).fold(1u128, |acc, i| acc * (n + 1 - i) / i)
    }

    #[test]
    fn the_null_distribution_is_exact() {
        for (m, n) in [(1, 1), (3, 4), (5, 5), (7, 10), (31, 31), (5, 31)] {
            let counts = mann_whitney_counts(m, n);
            assert_eq!(counts.len(), m * n + 1);
            let total: u128 = counts.iter().sum();
            assert_eq!(total, binomial((m + n) as u128, m as u128), "({m}, {n})");
            // Symmetric about m·n/2.
            for u in 0..counts.len() {
                assert_eq!(counts[u], counts[m * n - u], "({m}, {n}) u={u}");
            }
        }
        // By hand: m = n = 2 → U = 0..4 counts 1, 1, 2, 1, 1.
        assert_eq!(mann_whitney_counts(2, 2), vec![1, 1, 2, 1, 1]);
    }

    #[test]
    fn the_critical_values_and_confidences_are_the_designs() {
        for (n, c, conf) in [
            (5, 2, 96.8),
            (7, 8, 96.2),
            (10, 23, 95.7),
            (15, 64, 95.5),
            (20, 127, 95.1),
            (31, 341, 95.0),
        ] {
            assert_eq!(critical_value(n, n), Some(c), "n = {n}");
            let got = confidence(n, n).expect("a value") * 100.0;
            assert!((got - conf).abs() < 0.05, "n = {n}: {got}");
            // ±1 is not the critical value: c + 1 passes 0.025.
            let counts = mann_whitney_counts(n, n);
            let total: u128 = counts.iter().sum();
            let at = |u: usize| counts[..=u].iter().sum::<u128>();
            assert!(at(c) * 40 <= total && at(c + 1) * 40 > total, "n = {n}");
        }
        // Unequal sizes: at least 95 % at every (m, n) from 5 to 31.
        for m in 5..=31 {
            for n in 5..=31 {
                let got = confidence(m, n).expect("a value");
                assert!(got >= 0.95, "({m}, {n}): {got}");
            }
        }
        // Below 4 a side there is no interval at all.
        assert_eq!(critical_value(3, 3), None);
    }

    #[test]
    fn the_shift_drops_unusable_values_and_needs_five() {
        let base: Vec<Option<f64>> = (0..5).map(|i| Some(100.0 + i as f64)).collect();
        let mut other: Vec<Option<f64>> = (0..5).map(|i| Some(110.0 + i as f64)).collect();
        let s = hodges_lehmann(&base, &other).expect("five a side");
        assert!(s.lo <= s.estimate && s.estimate <= s.hi);
        assert!(s.percent() > 9.0 && s.percent() < 11.0, "{}", s.percent());
        other[0] = Some(0.0);
        assert!(
            hodges_lehmann(&base, &other).is_none(),
            "a zero drops its run"
        );
        other[0] = Some(f64::NAN);
        assert!(hodges_lehmann(&base, &other).is_none());
        other[0] = None;
        assert!(hodges_lehmann(&base, &other).is_none());
        other.push(Some(112.0));
        assert!(hodges_lehmann(&base, &other).is_some());
    }

    #[test]
    fn the_interval_narrows_as_runs_grow() {
        // A deterministic spread: the same quantiles at every n.
        let sample = |n: usize, scale: f64| -> Vec<Option<f64>> {
            (0..n)
                .map(|i| Some(scale * (1.0 + 0.03 * ((i as f64 + 0.5) / n as f64 - 0.5))))
                .collect()
        };
        let mut last = f64::INFINITY;
        for n in [5, 7, 10, 15, 20, 31] {
            let s = hodges_lehmann(&sample(n, 1.0), &sample(n, 1.05)).expect("values");
            let width = s.hi - s.lo;
            assert!(width < last, "n = {n}: {width} after {last}");
            last = width;
        }
    }

    #[test]
    fn spread_and_median() {
        assert_eq!(median(&[Some(3.0), None, Some(1.0), Some(2.0)]), Some(2.0));
        assert_eq!(median(&[None, Some(0.0)]), None);
        let s = robust_spread(&[1.0, 2.0, 3.0, 4.0, 100.0]).expect("five");
        assert!((s - 1.4826).abs() < 1e-9, "{s}");
        assert_eq!(robust_spread(&[1.0]), None);
    }
}
