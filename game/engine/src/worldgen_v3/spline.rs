//! Piecewise-linear splines and the polynomial helpers the v3 terrain is
//! shaped with (docs/foundations/2026-10-23-terrain-overhaul.md §4).
//!
//! Determinism (§0.2): only `+ - * /`, comparisons, `min`/`max`. Smoothstep is
//! the cubic polynomial, never `powf`.

/// `a` at `t = 0`, `b` at `t = 1`, linear between.
#[inline]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// `t` clamped to `[0, 1]`.
#[inline]
pub fn clamp01(t: f64) -> f64 {
    t.clamp(0.0, 1.0)
}

/// 0 at or below `e0`, 1 at or above `e1`, the cubic `t²(3 − 2t)` between.
#[inline]
pub fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = clamp01((x - e0) / (e1 - e0));
    t * t * (3.0 - 2.0 * t)
}

/// A piecewise-linear function through `(x, y)` knots (x ascending), flat
/// beyond the first and last knot.
pub struct Spline(pub &'static [(f64, f64)]);

impl Spline {
    pub fn eval(&self, x: f64) -> f64 {
        let k = self.0;
        if x <= k[0].0 {
            return k[0].1;
        }
        for w in k.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            if x < x1 {
                return lerp(y0, y1, (x - x0) / (x1 - x0));
            }
        }
        k[k.len() - 1].1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static S: Spline = Spline(&[(-1.0, 10.0), (0.0, 20.0), (1.0, 40.0)]);

    #[test]
    fn spline_hits_its_knots_and_clamps_outside() {
        assert_eq!(S.eval(-2.0), 10.0);
        assert_eq!(S.eval(-1.0), 10.0);
        assert_eq!(S.eval(0.0), 20.0);
        assert_eq!(S.eval(0.5), 30.0);
        assert_eq!(S.eval(1.0), 40.0);
        assert_eq!(S.eval(9.0), 40.0);
    }

    #[test]
    fn smoothstep_is_flat_at_both_ends() {
        assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
        assert_eq!(smoothstep(0.0, 1.0, 0.5), 0.5);
        assert!(smoothstep(-0.12, 0.05, 0.0) > 0.5);
    }
}
