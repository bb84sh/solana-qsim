//! Fixed-point quantum circuit simulator for Solana programs.
//!
//! A state vector of `2^n` complex amplitudes in Q1.30 fixed point (8 bytes each), gates from
//! the common OpenQASM set, and scores that compare a quantum computer's measured counts with
//! the ideal distribution. No floats and no allocation: the state is any `&mut [Complex]`, so it
//! can live on the heap or directly in account data. Results are bit-for-bit deterministic.
//!
//! Qubit `q` is bit `q` of the basis-state index (Qiskit's little-endian order).
#![no_std]

pub mod fixed;
pub mod gate;
pub mod score;
pub mod state;

pub use fixed::Complex;
pub use gate::{Circuit, Gate};
pub use score::{score, score_amplitudes, Count, Score};
pub use state::State;

/// Largest supported register: 2^20 amplitudes = 8 MiB, under Solana's 10 MiB account limit.
pub const MAX_QUBITS: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The amplitude buffer is not a power of two, or holds more than `MAX_QUBITS` qubits.
    BadStateLength,
    /// The circuit bytes end in the middle of a gate.
    Truncated,
    /// Unknown gate opcode.
    UnknownGate(u8),
    /// A qubit index is outside the register.
    QubitOutOfRange(u8),
    /// A multi-qubit gate uses the same qubit twice.
    RepeatedQubit(u8),
    /// A work-item range is outside the gate's work items.
    BadRange,
    /// Counts are empty, not strictly increasing, or name an outcome outside the register.
    BadCounts,
}
