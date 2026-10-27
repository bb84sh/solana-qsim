//! The state vector and gate application.
//!
//! A gate on k qubits touches 2^n amplitudes in 2^(n-k) independent groups ("work items").
//! `apply_range` runs any slice of those groups, so one gate on a large register can be split
//! across transactions; running every range once in any order gives the same state as `apply`.

use crate::{
    fixed::{cos_sin, mul, Complex, FRAC_1_SQRT_2, ONE},
    gate::{Circuit, Gate},
    Error, MAX_QUBITS,
};

pub struct State<'a> {
    amps: &'a mut [Complex],
    n: u32,
}

/// Inserts a 0 bit at position `bit` of `j` (bits at and above `bit` move up one place).
#[inline(always)]
fn insert_zero(j: usize, bit: u32) -> usize {
    let low = (1usize << bit) - 1;
    ((j & !low) << 1) | (j & low)
}

/// How a one-qubit gate transforms each pair (v0, v1) of amplitudes that differ in that qubit.
#[derive(Clone, Copy)]
enum OneQubit {
    Swap,
    NegOne,
    /// v1 ×= i
    MulI,
    /// v1 ×= -i
    MulNegI,
    /// v1 ×= c
    PhaseOne(Complex),
    /// v0 ×= c0, v1 ×= c1
    Diagonal(Complex, Complex),
    Hadamard,
    PauliY,
    /// [[m00, m01], [m10, m11]]
    Matrix([Complex; 4]),
}

impl<'a> State<'a> {
    /// Wraps an amplitude buffer of length 2^n (n ≤ `MAX_QUBITS`). The buffer is used as is;
    /// call [`State::reset`] to start from |0…0⟩.
    pub fn new(amps: &'a mut [Complex]) -> Result<Self, Error> {
        let len = amps.len();
        if !len.is_power_of_two() || len.trailing_zeros() > MAX_QUBITS {
            return Err(Error::BadStateLength);
        }
        Ok(State { n: len.trailing_zeros(), amps })
    }

    pub fn num_qubits(&self) -> u32 {
        self.n
    }

    pub fn amplitudes(&self) -> &[Complex] {
        self.amps
    }

    /// |0…0⟩.
    pub fn reset(&mut self) {
        self.amps.fill(Complex::ZERO);
        self.amps[0] = Complex::ONE;
    }

    /// Probability of measuring basis state `index`, in Q60.
    pub fn probability(&self, index: usize) -> u64 {
        self.amps[index].norm_sqr()
    }

    /// Number of independent amplitude groups the gate touches.
    pub fn work_items(&self, gate: &Gate) -> usize {
        1usize << (self.n as usize - gate.qubits().1)
    }

    /// Applies every gate of an encoded circuit; returns how many gates ran.
    pub fn run(&mut self, circuit: Circuit) -> Result<u32, Error> {
        let mut count = 0;
        for gate in circuit {
            self.apply(&gate?)?;
            count += 1;
        }
        Ok(count)
    }

    pub fn apply(&mut self, gate: &Gate) -> Result<(), Error> {
        self.apply_range(gate, 0, self.work_items(gate))
    }

    /// Applies the gate to work items `start..end` only.
    pub fn apply_range(&mut self, gate: &Gate, start: usize, end: usize) -> Result<(), Error> {
        gate.validate(self.n)?;
        if start > end || end > self.work_items(gate) {
            return Err(Error::BadRange);
        }
        use Gate::*;
        match *gate {
            H(q) => self.one_qubit(q, OneQubit::Hadamard, start, end),
            X(q) => self.one_qubit(q, OneQubit::Swap, start, end),
            Y(q) => self.one_qubit(q, OneQubit::PauliY, start, end),
            Z(q) => self.one_qubit(q, OneQubit::NegOne, start, end),
            S(q) => self.one_qubit(q, OneQubit::MulI, start, end),
            Sdg(q) => self.one_qubit(q, OneQubit::MulNegI, start, end),
            T(q) => self.one_qubit(q, OneQubit::PhaseOne(Complex::new(FRAC_1_SQRT_2, FRAC_1_SQRT_2)), start, end),
            Tdg(q) => self.one_qubit(q, OneQubit::PhaseOne(Complex::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2)), start, end),
            Sx(q) => {
                let (p, m) = (Complex::new(ONE / 2, ONE / 2), Complex::new(ONE / 2, -ONE / 2));
                self.one_qubit(q, OneQubit::Matrix([p, m, m, p]), start, end)
            }
            Rx(q, theta) => {
                let (c, s) = cos_sin(theta >> 1);
                let (c, ms) = (Complex::new(c, 0), Complex::new(0, -s));
                self.one_qubit(q, OneQubit::Matrix([c, ms, ms, c]), start, end)
            }
            Ry(q, theta) => {
                let (c, s) = cos_sin(theta >> 1);
                let m = [Complex::new(c, 0), Complex::new(-s, 0), Complex::new(s, 0), Complex::new(c, 0)];
                self.one_qubit(q, OneQubit::Matrix(m), start, end)
            }
            Rz(q, theta) => {
                let half = theta >> 1;
                self.one_qubit(q, OneQubit::Diagonal(Complex::cis(half.wrapping_neg()), Complex::cis(half)), start, end)
            }
            P(q, lambda) => self.one_qubit(q, OneQubit::PhaseOne(Complex::cis(lambda)), start, end),
            U(q, theta, phi, lambda) => {
                let (c, s) = cos_sin(theta >> 1);
                let (c, s) = (c as i64, s as i64);
                let (el, ep, epl) = (Complex::cis(lambda), Complex::cis(phi), Complex::cis(phi.wrapping_add(lambda)));
                let m = [
                    Complex::new(c as i32, 0),
                    Complex::new(-mul(el.re as i64, s), -mul(el.im as i64, s)),
                    Complex::new(mul(ep.re as i64, s), mul(ep.im as i64, s)),
                    Complex::new(mul(epl.re as i64, c), mul(epl.im as i64, c)),
                ];
                self.one_qubit(q, OneQubit::Matrix(m), start, end)
            }
            Cx(c, t) => {
                let (cb, tb) = (1usize << c, 1usize << t);
                let amps = &mut *self.amps;
                runs2(start, end, c, t, |i, len| swap_runs(amps, i | cb, (i | cb) | tb, len));
            }
            Cz(a, b) => {
                let both = (1usize << a) | (1usize << b);
                let amps = &mut *self.amps;
                runs2(start, end, a, b, |i, len| amps[i | both..(i | both) + len].iter_mut().for_each(|v| *v = v.neg()));
            }
            Swap(a, b) => {
                let (ab, bb) = (1usize << a, 1usize << b);
                let amps = &mut *self.amps;
                runs2(start, end, a, b, |i, len| swap_runs(amps, i | ab, i | bb, len));
            }
            Cp(a, b, lambda) => {
                let both = (1usize << a) | (1usize << b);
                let phase = Complex::cis(lambda);
                let amps = &mut *self.amps;
                runs2(start, end, a, b, |i, len| amps[i | both..(i | both) + len].iter_mut().for_each(|v| *v = v.mul(phase)));
            }
            Ccx(c1, c2, t) => {
                let (controls, tb) = ((1usize << c1) | (1usize << c2), 1usize << t);
                let mut q = [c1 as u32, c2 as u32, t as u32];
                q.sort_unstable();
                let amps = &mut *self.amps;
                let run = 1usize << q[0];
                let mut j = start;
                while j < end {
                    let take = (run - (j & (run - 1))).min(end - j);
                    let i = insert_zero(insert_zero(insert_zero(j, q[0]), q[1]), q[2]) | controls;
                    swap_runs(amps, i, i | tb, take);
                    j += take;
                }
            }
        }
        Ok(())
    }

    /// Pairs (i, i + 2^q) sit in blocks: within a block of 2^(q+1) amplitudes the first half
    /// pairs with the second half. Work item j is offset (j mod 2^q) in block (j >> q).
    fn one_qubit(&mut self, q: u8, kind: OneQubit, start: usize, end: usize) {
        match kind {
            OneQubit::Swap => self.each_pair(q, start, end, |a, b| core::mem::swap(a, b)),
            OneQubit::NegOne => self.each_pair(q, start, end, |_, b| *b = b.neg()),
            OneQubit::MulI => self.each_pair(q, start, end, |_, b| *b = Complex::new(-b.im, b.re)),
            OneQubit::MulNegI => self.each_pair(q, start, end, |_, b| *b = Complex::new(b.im, -b.re)),
            OneQubit::PhaseOne(c) => self.each_pair(q, start, end, |_, b| *b = b.mul(c)),
            OneQubit::Diagonal(c0, c1) => self.each_pair(q, start, end, |a, b| {
                *a = a.mul(c0);
                *b = b.mul(c1);
            }),
            OneQubit::Hadamard => {
                let k = FRAC_1_SQRT_2 as i64;
                self.each_pair(q, start, end, |a, b| {
                    let (v0, v1) = (*a, *b);
                    *a = Complex::new(mul(v0.re as i64 + v1.re as i64, k), mul(v0.im as i64 + v1.im as i64, k));
                    *b = Complex::new(mul(v0.re as i64 - v1.re as i64, k), mul(v0.im as i64 - v1.im as i64, k));
                })
            }
            OneQubit::PauliY => self.each_pair(q, start, end, |a, b| {
                let (v0, v1) = (*a, *b);
                *a = Complex::new(v1.im, -v1.re);
                *b = Complex::new(-v0.im, v0.re);
            }),
            OneQubit::Matrix([m00, m01, m10, m11]) => self.each_pair(q, start, end, |a, b| {
                let (v0, v1) = (*a, *b);
                *a = Complex::mul_add(m00, v0, m01, v1);
                *b = Complex::mul_add(m10, v0, m11, v1);
            }),
        }
    }

    /// Calls `f` on every pair of work items `start..end`: a partial first block, whole blocks
    /// walked with `chunks_exact_mut` (no index maths or bounds checks per pair), a partial last
    /// block. Generic over `f`, so each gate gets its own tight loop.
    #[inline(always)]
    fn each_pair(&mut self, q: u8, start: usize, end: usize, mut f: impl FnMut(&mut Complex, &mut Complex)) {
        let bit = 1usize << q;
        let block_of = |j: usize| ((j >> q) << (q + 1)) + (j & (bit - 1));
        let mut j = start;
        fn partial<F: FnMut(&mut Complex, &mut Complex)>(amps: &mut [Complex], base: usize, bit: usize, take: usize, f: &mut F) {
            let (lo, hi) = amps[base..base + bit + take].split_at_mut(bit);
            lo[..take].iter_mut().zip(hi[..take].iter_mut()).for_each(|(a, b)| f(a, b));
        }
        if j & (bit - 1) != 0 && j < end {
            let take = (bit - (j & (bit - 1))).min(end - j);
            partial(self.amps, block_of(j), bit, take, &mut f);
            j += take;
        }
        let whole = (end - j) >> q;
        if whole > 0 {
            let base = block_of(j);
            for block in self.amps[base..base + (whole << (q + 1))].chunks_exact_mut(bit << 1) {
                let (lo, hi) = block.split_at_mut(bit);
                lo.iter_mut().zip(hi.iter_mut()).for_each(|(a, b)| f(a, b));
            }
            j += whole << q;
        }
        if j < end {
            partial(self.amps, block_of(j), bit, end - j, &mut f);
        }
    }

}

/// Splits two-qubit work items `start..end` into runs of consecutive amplitude indices (the
/// lower of the two qubits sets the run length); calls `f(first index, run length)` per run.
#[inline(always)]
fn runs2(start: usize, end: usize, a: u8, b: u8, mut f: impl FnMut(usize, usize)) {
    let (lo, hi) = if a < b { (a as u32, b as u32) } else { (b as u32, a as u32) };
    if lo == 0 {
        // Runs of one: skip the run bookkeeping.
        for j in start..end {
            f(insert_zero(insert_zero(j, 0), hi), 1);
        }
        return;
    }
    let run = 1usize << lo;
    let mut j = start;
    while j < end {
        let take = (run - (j & (run - 1))).min(end - j);
        f(insert_zero(insert_zero(j, lo), hi), take);
        j += take;
    }
}

/// Swaps amps[x..x+len] with amps[y..y+len]. The runs never overlap: x and y differ in a qubit
/// bit at or above the run length.
#[inline(always)]
fn swap_runs(amps: &mut [Complex], x: usize, y: usize, len: usize) {
    if len == 1 {
        amps.swap(x, y);
        return;
    }
    let (x, y) = if x < y { (x, y) } else { (y, x) };
    let (left, right) = amps.split_at_mut(y);
    left[x..x + len].swap_with_slice(&mut right[..len]);
}
