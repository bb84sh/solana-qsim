//! Checks the fixed-point simulator against an independent f64 reference built from the
//! textbook gate matrices (Qiskit conventions).

use solana_qsim::{
    fixed::{cos_sin, isqrt, ONE},
    gate::Gate,
    score, Circuit, Complex, Count, State,
};
use std::f64::consts::PI;

type C = (f64, f64);
const I: C = (0.0, 1.0);

fn cmul(a: C, b: C) -> C {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}
fn cadd(a: C, b: C) -> C {
    (a.0 + b.0, a.1 + b.1)
}
fn cis(x: f64) -> C {
    (x.cos(), x.sin())
}
fn rad(turns: u32) -> f64 {
    turns as f64 * 2.0 * PI / 4294967296.0
}

/// Reference: apply a 2x2 matrix to qubit q.
fn ref_one(state: &mut [C], q: u8, m: [C; 4]) {
    let bit = 1 << q;
    for i in 0..state.len() {
        if i & bit == 0 {
            let (v0, v1) = (state[i], state[i | bit]);
            state[i] = cadd(cmul(m[0], v0), cmul(m[1], v1));
            state[i | bit] = cadd(cmul(m[2], v0), cmul(m[3], v1));
        }
    }
}

fn ref_apply(state: &mut [C], gate: &Gate) {
    let s = 1.0 / 2f64.sqrt();
    let (z, o) = ((0.0, 0.0), (1.0, 0.0));
    let neg = |c: C| (-c.0, -c.1);
    match *gate {
        Gate::H(q) => ref_one(state, q, [(s, 0.0), (s, 0.0), (s, 0.0), (-s, 0.0)]),
        Gate::X(q) => ref_one(state, q, [z, o, o, z]),
        Gate::Y(q) => ref_one(state, q, [z, neg(I), I, z]),
        Gate::Z(q) => ref_one(state, q, [o, z, z, neg(o)]),
        Gate::S(q) => ref_one(state, q, [o, z, z, I]),
        Gate::Sdg(q) => ref_one(state, q, [o, z, z, neg(I)]),
        Gate::T(q) => ref_one(state, q, [o, z, z, cis(PI / 4.0)]),
        Gate::Tdg(q) => ref_one(state, q, [o, z, z, cis(-PI / 4.0)]),
        Gate::Sx(q) => ref_one(state, q, [(0.5, 0.5), (0.5, -0.5), (0.5, -0.5), (0.5, 0.5)]),
        Gate::Rx(q, t) => {
            let (c, sn) = ((rad(t) / 2.0).cos(), (rad(t) / 2.0).sin());
            ref_one(state, q, [(c, 0.0), (0.0, -sn), (0.0, -sn), (c, 0.0)])
        }
        Gate::Ry(q, t) => {
            let (c, sn) = ((rad(t) / 2.0).cos(), (rad(t) / 2.0).sin());
            ref_one(state, q, [(c, 0.0), (-sn, 0.0), (sn, 0.0), (c, 0.0)])
        }
        Gate::Rz(q, t) => ref_one(state, q, [cis(-rad(t) / 2.0), z, z, cis(rad(t) / 2.0)]),
        Gate::P(q, l) => ref_one(state, q, [o, z, z, cis(rad(l))]),
        Gate::U(q, t, p, l) => {
            let (c, sn) = ((rad(t) / 2.0).cos(), (rad(t) / 2.0).sin());
            let m = [(c, 0.0), neg(cmul(cis(rad(l)), (sn, 0.0))), cmul(cis(rad(p)), (sn, 0.0)), cmul(cis(rad(p) + rad(l)), (c, 0.0))];
            ref_one(state, q, m)
        }
        Gate::Cx(c, t) => {
            for i in 0..state.len() {
                if i >> c & 1 == 1 && i >> t & 1 == 0 {
                    state.swap(i, i | 1 << t);
                }
            }
        }
        Gate::Cz(a, b) => {
            for (i, v) in state.iter_mut().enumerate() {
                if i >> a & 1 == 1 && i >> b & 1 == 1 {
                    *v = neg(*v);
                }
            }
        }
        Gate::Swap(a, b) => {
            for i in 0..state.len() {
                if i >> a & 1 == 1 && i >> b & 1 == 0 {
                    state.swap(i, i ^ (1 << a) ^ (1 << b));
                }
            }
        }
        Gate::Cp(a, b, l) => {
            for (i, v) in state.iter_mut().enumerate() {
                if i >> a & 1 == 1 && i >> b & 1 == 1 {
                    *v = cmul(*v, cis(rad(l)));
                }
            }
        }
        Gate::Ccx(c1, c2, t) => {
            for i in 0..state.len() {
                if i >> c1 & 1 == 1 && i >> c2 & 1 == 1 && i >> t & 1 == 0 {
                    state.swap(i, i | 1 << t);
                }
            }
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// k distinct qubits below n.
    fn qubits(&mut self, n: u32, k: usize) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        while out.len() < k {
            let q = self.below(n as u64) as u8;
            if !out.contains(&q) {
                out.push(q);
            }
        }
        out
    }
}

fn random_gate(rng: &mut Rng, n: u32) -> Gate {
    let a = rng.next() as u32;
    loop {
        let kind = rng.below(19);
        if (kind >= 14 && n < 2) || (kind == 18 && n < 3) {
            continue;
        }
        let q = rng.qubits(n, if kind >= 18 { 3 } else if kind >= 14 { 2 } else { 1 });
        return match kind {
            0 => Gate::H(q[0]),
            1 => Gate::X(q[0]),
            2 => Gate::Y(q[0]),
            3 => Gate::Z(q[0]),
            4 => Gate::S(q[0]),
            5 => Gate::Sdg(q[0]),
            6 => Gate::T(q[0]),
            7 => Gate::Tdg(q[0]),
            8 => Gate::Sx(q[0]),
            9 => Gate::Rx(q[0], a),
            10 => Gate::Ry(q[0], a),
            11 => Gate::Rz(q[0], a),
            12 => Gate::P(q[0], a),
            13 => Gate::U(q[0], a, a.rotate_left(11), a.rotate_left(23)),
            14 => Gate::Cx(q[0], q[1]),
            15 => Gate::Cz(q[0], q[1]),
            16 => Gate::Swap(q[0], q[1]),
            17 => Gate::Cp(q[0], q[1], a),
            _ => Gate::Ccx(q[0], q[1], q[2]),
        };
    }
}

fn max_error(fixed: &[Complex], reference: &[C]) -> f64 {
    let one = ONE as f64;
    fixed
        .iter()
        .zip(reference)
        .map(|(f, r)| (f.re as f64 / one - r.0).abs().max((f.im as f64 / one - r.1).abs()))
        .fold(0.0, f64::max)
}

#[test]
fn cos_sin_matches_f64() {
    let mut worst = 0.0f64;
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let special = [0u32, 1 << 29, 1 << 30, 3 << 29, 1 << 31, 3 << 30, u32::MAX, 1, 12345];
    for t in special.into_iter().chain((0..200_000).map(|_| rng.next() as u32)) {
        let (c, s) = cos_sin(t);
        let x = rad(t);
        worst = worst.max((c as f64 / ONE as f64 - x.cos()).abs()).max((s as f64 / ONE as f64 - x.sin()).abs());
    }
    println!("cos_sin worst error: {worst:.3e} ({:.2} ulp of 2^-30)", worst * ONE as f64);
    assert!(worst < 4.0 / ONE as f64, "{worst}");
    // Quarter turns are exact.
    assert_eq!(cos_sin(0), (ONE, 0));
    assert_eq!(cos_sin(1 << 30), (0, ONE));
    assert_eq!(cos_sin(1 << 31), (-ONE, 0));
    assert_eq!(cos_sin(3 << 30), (0, -ONE));
}

#[test]
fn isqrt_is_floor_sqrt() {
    let mut rng = Rng(42);
    for n in (0..10_000u64).chain((0..10_000).map(|_| rng.next())).chain([u64::MAX, 1 << 60, (1 << 60) - 1]) {
        let r = isqrt(n);
        assert!(r as u128 * r as u128 <= n as u128 && (r as u128 + 1) * (r as u128 + 1) > n as u128, "{n}");
    }
}

#[test]
fn random_circuits_match_reference() {
    let mut rng = Rng(0xB884_2026);
    let mut worst = 0.0f64;
    for n in 1..=8u32 {
        for _ in 0..25 {
            let gates: Vec<Gate> = (0..80).map(|_| random_gate(&mut rng, n)).collect();
            // Through the byte encoding, as a program would receive it.
            let mut bytes = vec![0u8; gates.iter().map(|g| g.encoded_len()).sum()];
            let mut at = 0;
            for g in &gates {
                at += g.encode(&mut bytes[at..]);
            }
            let decoded: Vec<Gate> = Circuit(&bytes).map(|g| g.unwrap()).collect();
            assert_eq!(decoded, gates);

            let mut amps = vec![Complex::ZERO; 1 << n];
            let mut state = State::new(&mut amps).unwrap();
            state.reset();
            assert_eq!(state.run(Circuit(&bytes)).unwrap(), 80);

            let mut reference = vec![(0.0, 0.0); 1 << n];
            reference[0] = (1.0, 0.0);
            for g in &gates {
                ref_apply(&mut reference, g);
            }
            worst = worst.max(max_error(&amps, &reference));
        }
    }
    println!("80-gate random circuits, n = 1..8: worst amplitude error {worst:.3e}");
    assert!(worst < 1e-6, "{worst}");
}

#[test]
fn split_ranges_equal_whole_gate() {
    let mut rng = Rng(7);
    let n = 7;
    for _ in 0..200 {
        let gate = random_gate(&mut rng, n);
        let mut init: Vec<Complex> = vec![Complex::ZERO; 1 << n];
        let mut seed = State::new(&mut init).unwrap();
        seed.reset();
        for _ in 0..10 {
            seed.apply(&random_gate(&mut rng, n)).unwrap();
        }
        let mut whole = init.clone();
        State::new(&mut whole).unwrap().apply(&gate).unwrap();
        let mut split = init.clone();
        let mut state = State::new(&mut split).unwrap();
        let items = state.work_items(&gate);
        let cut = rng.below(items as u64 + 1) as usize;
        state.apply_range(&gate, cut, items).unwrap();
        state.apply_range(&gate, 0, cut).unwrap();
        assert_eq!(whole, split, "{gate:?}");
    }
}

#[test]
fn invalid_input_is_rejected() {
    use solana_qsim::Error;
    let mut amps = vec![Complex::ZERO; 8];
    let mut state = State::new(&mut amps).unwrap();
    assert_eq!(state.apply(&Gate::H(3)), Err(Error::QubitOutOfRange(3)));
    assert_eq!(state.apply(&Gate::Cx(1, 1)), Err(Error::RepeatedQubit(1)));
    assert_eq!(state.apply_range(&Gate::H(0), 0, 5), Err(Error::BadRange));
    assert_eq!(Circuit(&[0x99]).next(), Some(Err(Error::UnknownGate(0x99))));
    assert_eq!(Circuit(&[0x10, 0, 1, 2]).next(), Some(Err(Error::Truncated)));
    assert!(State::new(&mut vec![Complex::ZERO; 6]).is_err());
}

/// Scores against f64 formulas, for a GHZ state and a random state.
#[test]
fn scores_match_reference() {
    // GHZ on 3 qubits: ideal p(000) = p(111) = 1/2.
    let mut amps = vec![Complex::ZERO; 8];
    let mut ghz = State::new(&mut amps).unwrap();
    ghz.reset();
    for g in [Gate::H(0), Gate::Cx(0, 1), Gate::Cx(1, 2)] {
        ghz.apply(&g).unwrap();
    }
    let ideal = [Count { outcome: 0, count: 500 }, Count { outcome: 7, count: 500 }];
    let s = score(&ghz, &ideal).unwrap();
    assert!(s.fidelity_ppm >= 999_990 && s.tvd_ppm <= 10, "{s:?}");
    assert!((s.xeb_ppm - 3_000_000).abs() <= 10, "{s:?}"); // 2^3 · ½ − 1 = 3
    // A noisy run: some bit flips.
    let noisy = [
        Count { outcome: 0, count: 450 },
        Count { outcome: 1, count: 30 },
        Count { outcome: 6, count: 20 },
        Count { outcome: 7, count: 500 },
    ];
    let s = score(&ghz, &noisy).unwrap();
    let q: [f64; 4] = [0.45, 0.03, 0.02, 0.5];
    let p: [f64; 4] = [0.5, 0.0, 0.0, 0.5];
    let fid: f64 = p.iter().zip(&q).map(|(p, q)| (p * q).sqrt()).sum::<f64>().powi(2);
    let tvd: f64 = p.iter().zip(&q).map(|(p, q)| (p - q).abs()).sum::<f64>() / 2.0;
    let xeb = 8.0 * (0.45 * 0.5 + 0.5 * 0.5) - 1.0;
    println!("noisy GHZ: {s:?} (f64: fidelity {fid:.6}, tvd {tvd:.6}, xeb {xeb:.6})");
    assert!((s.fidelity_ppm as f64 / 1e6 - fid).abs() < 2e-6);
    assert!((s.tvd_ppm as f64 / 1e6 - tvd).abs() < 2e-6);
    assert!((s.xeb_ppm as f64 / 1e6 - xeb).abs() < 2e-6);
    // A uniform (pure noise) sample scores about 0 on XEB and 0.25 fidelity here.
    let uniform: Vec<Count> = (0..8).map(|o| Count { outcome: o, count: 125 }).collect();
    let s = score(&ghz, &uniform).unwrap();
    assert!(s.xeb_ppm.abs() <= 10 && (s.fidelity_ppm as i64 - 250_000).abs() <= 10, "{s:?}");
    // Bad counts.
    assert!(score(&ghz, &[]).is_err());
    assert!(score(&ghz, &[Count { outcome: 7, count: 1 }, Count { outcome: 0, count: 1 }]).is_err());
    assert!(score(&ghz, &[Count { outcome: 8, count: 1 }]).is_err());
}
