pub mod simulate;
pub mod state_ops;

pub use simulate::*;
pub use state_ops::*;

use anchor_lang::prelude::*;

/// One observed outcome: the measured bitstring (qubit q = bit q) and how many shots gave it.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct Count {
    pub outcome: u32,
    pub count: u32,
}

/// How well measured counts match the circuit's ideal distribution (all in parts per million).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct QsimScore {
    pub num_qubits: u8,
    pub shots: u64,
    pub outcomes: u32,
    /// Hellinger fidelity (Qiskit's `hellinger_fidelity`); 1,000,000 = identical.
    pub fidelity_ppm: u32,
    /// Total variation distance; 0 = identical.
    pub tvd_ppm: u32,
    /// Linear cross-entropy benchmark; ~0 for uniform noise.
    pub xeb_ppm: i64,
    /// sha256 chain over the gates that produced the state.
    pub circuit_hash: [u8; 32],
}

pub(crate) fn to_counts(counts: &[Count]) -> Vec<solana_qsim::Count> {
    counts.iter().map(|c| solana_qsim::Count { outcome: c.outcome, count: c.count }).collect()
}

pub(crate) fn report(num_qubits: u8, s: solana_qsim::Score, circuit_hash: [u8; 32]) -> QsimScore {
    msg!(
        "qsim {} qubits, {} shots: fidelity {} ppm, tvd {} ppm, xeb {} ppm",
        num_qubits, s.shots, s.fidelity_ppm, s.tvd_ppm, s.xeb_ppm
    );
    QsimScore {
        num_qubits,
        shots: s.shots,
        outcomes: s.outcomes,
        fidelity_ppm: s.fidelity_ppm,
        tvd_ppm: s.tvd_ppm,
        xeb_ppm: s.xeb_ppm,
        circuit_hash,
    }
}
