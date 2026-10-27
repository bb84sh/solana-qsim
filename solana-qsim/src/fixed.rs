//! Q1.30 fixed-point numbers: `x` represents `x / 2^30`, so amplitudes in [-1, 1] fit an `i32`.
//! Products go through `i64` and round to nearest.

pub const FRAC_BITS: u32 = 30;
pub const ONE: i32 = 1 << FRAC_BITS;
const HALF: i64 = 1 << (FRAC_BITS - 1);
/// round(2^30 / sqrt(2))
pub const FRAC_1_SQRT_2: i32 = 759_250_125;
/// round(2 * pi * 2^30)
const TWO_PI_Q30: u64 = 6_746_518_852;

/// Q30 × Q30 → Q30, rounded.
#[inline(always)]
pub fn mul(a: i64, b: i64) -> i32 {
    ((a * b + HALF) >> FRAC_BITS) as i32
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Complex {
    pub re: i32,
    pub im: i32,
}

impl Complex {
    pub const ZERO: Complex = Complex { re: 0, im: 0 };
    pub const ONE: Complex = Complex { re: ONE, im: 0 };

    #[inline(always)]
    pub const fn new(re: i32, im: i32) -> Self {
        Complex { re, im }
    }

    /// self × other, rounded.
    #[inline(always)]
    pub fn mul(self, other: Complex) -> Complex {
        let (a, b, c, d) = (self.re as i64, self.im as i64, other.re as i64, other.im as i64);
        Complex { re: ((a * c - b * d + HALF) >> FRAC_BITS) as i32, im: ((a * d + b * c + HALF) >> FRAC_BITS) as i32 }
    }

    /// a×x + b×y for complex coefficients, rounded once.
    #[inline(always)]
    pub fn mul_add(a: Complex, x: Complex, b: Complex, y: Complex) -> Complex {
        let re = a.re as i64 * x.re as i64 - a.im as i64 * x.im as i64 + b.re as i64 * y.re as i64 - b.im as i64 * y.im as i64;
        let im = a.re as i64 * x.im as i64 + a.im as i64 * x.re as i64 + b.re as i64 * y.im as i64 + b.im as i64 * y.re as i64;
        Complex { re: ((re + HALF) >> FRAC_BITS) as i32, im: ((im + HALF) >> FRAC_BITS) as i32 }
    }

    /// |z|² in Q60.
    #[inline(always)]
    pub fn norm_sqr(self) -> u64 {
        (self.re as i64 * self.re as i64 + self.im as i64 * self.im as i64) as u64
    }

    #[inline(always)]
    pub fn neg(self) -> Complex {
        Complex { re: -self.re, im: -self.im }
    }

    /// e^{i·angle}, angle in turns (see [`cos_sin`]).
    pub fn cis(turns: u32) -> Complex {
        let (c, s) = cos_sin(turns);
        Complex { re: c, im: s }
    }
}

/// (cos θ, sin θ) in Q30 for θ = 2π · turns / 2^32. Multiples of a quarter turn are exact;
/// elsewhere the error is about 2^-29.
pub fn cos_sin(turns: u32) -> (i32, i32) {
    let quadrant = turns >> 30;
    let r = turns & 0x3FFF_FFFF; // angle within the quadrant, in units of 2^-32 turns
    // Reduce to [0, π/4]: use the complement for the upper half of the quadrant.
    let (c, s) = if r <= 1 << 29 {
        cos_sin_small(r)
    } else {
        let (c, s) = cos_sin_small((1 << 30) - r);
        (s, c)
    };
    match quadrant {
        0 => (c, s),
        1 => (-s, c),
        2 => (-c, -s),
        _ => (s, -c),
    }
}

/// Taylor series for x in [0, π/4] (r ≤ 2^29 turn units); truncation error below 2^-32.
fn cos_sin_small(r: u32) -> (i32, i32) {
    if r == 0 {
        return (ONE, 0);
    }
    let x = ((r as u64 * TWO_PI_Q30) >> 32) as i64; // radians, Q30
    let x2 = ((x * x + HALF) >> FRAC_BITS) as i64;
    let one = ONE as i64;
    // Horner form: sin x = x(1 - x²/6(1 - x²/20(1 - x²/42(1 - x²/72(1 - x²/110)))))
    let mut s = one;
    for d in [110, 72, 42, 20, 6] {
        s = one - (x2 * s / d + HALF) / one;
    }
    let sin = mul(x, s);
    // cos x = 1 - x²/2(1 - x²/12(1 - x²/30(1 - x²/56(1 - x²/90(1 - x²/132)))))
    let mut c = one;
    for d in [132, 90, 56, 30, 12, 2] {
        c = one - (x2 * c / d + HALF) / one;
    }
    (c as i32, sin)
}

/// Integer square root (floor) of a u64.
pub fn isqrt(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    // Start above the root: 2^ceil(bits/2), then Newton steps down to the floor.
    let mut x = 1u64 << ((64 - n.leading_zeros()).div_ceil(2));
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}
