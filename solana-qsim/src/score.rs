//! Scores measured counts (e.g. from a QPU) against the state's ideal distribution.
//!
//! Only outcomes that were observed are read, so scoring costs O(distinct outcomes), not O(2^n).
//! All results are in parts per million (ppm).

use crate::{fixed::isqrt, state::State, Complex, Error};

/// One observed outcome: the measured bitstring as an integer (qubit q = bit q) and how often.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Count {
    pub outcome: u32,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Score {
    pub shots: u64,
    pub outcomes: u32,
    /// Hellinger fidelity (Σ √(p·q))², as Qiskit's `hellinger_fidelity`. 1,000,000 = identical.
    pub fidelity_ppm: u32,
    /// Total variation distance ½ Σ |p − q|. 0 = identical, 1,000,000 = disjoint.
    pub tvd_ppm: u32,
    /// Linear cross-entropy benchmark 2^n · mean(p(observed)) − 1. About 0 for uniform noise;
    /// higher means the samples favour the outcomes the circuit makes likely.
    pub xeb_ppm: i64,
}

const ONE_Q60: u128 = 1 << 60;

fn ppm_q60(x: u128) -> u64 {
    ((x * 1_000_000 + (ONE_Q60 >> 1)) >> 60) as u64
}

/// `counts` must be non-empty, with strictly increasing outcomes inside the register.
pub fn score(state: &State, counts: &[Count]) -> Result<Score, Error> {
    score_amplitudes(state.amplitudes(), counts)
}

/// [`score`] for a read-only amplitude slice (length 2^n), e.g. borrowed from account data.
pub fn score_amplitudes(amps: &[Complex], counts: &[Count]) -> Result<Score, Error> {
    if !amps.len().is_power_of_two() {
        return Err(Error::BadStateLength);
    }
    let n = amps.len().trailing_zeros();
    let shots: u64 = counts.iter().map(|c| c.count as u64).sum();
    if shots == 0 || counts.windows(2).any(|w| w[0].outcome >= w[1].outcome) || counts.iter().any(|c| (c.outcome as u64) >> n != 0) {
        return Err(Error::BadCounts);
    }

    let (mut bc, mut abs_diff, mut p_seen, mut weighted) = (0u128, 0u128, 0u128, 0u128);
    for c in counts {
        let p = amps[c.outcome as usize].norm_sqr(); // Q60
        let q = (((c.count as u128) << 60) / shots as u128) as u64; // Q60
        bc += isqrt(p) as u128 * isqrt(q) as u128; // √p·√q, Q30·Q30 = Q60
        abs_diff += p.abs_diff(q) as u128;
        p_seen += p as u128;
        weighted += c.count as u128 * p as u128;
    }
    // Probability the ideal state puts on outcomes nobody observed (q = 0 there).
    let unseen = ONE_Q60.saturating_sub(p_seen);
    let tvd = (abs_diff + unseen) / 2;
    let bc = bc.min(ONE_Q60);
    let fidelity = (bc * bc) >> 60;
    let mean_p = weighted / shots as u128; // Q60
    let xeb = ((mean_p << n) as i128 - ONE_Q60 as i128) * 1_000_000 / ONE_Q60 as i128;

    Ok(Score {
        shots,
        outcomes: counts.len() as u32,
        fidelity_ppm: ppm_q60(fidelity) as u32,
        tvd_ppm: ppm_q60(tvd.min(ONE_Q60)) as u32,
        xeb_ppm: xeb as i64,
    })
}
