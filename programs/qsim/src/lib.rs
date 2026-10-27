pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use instructions::*;

declare_id!("6sHufVFfaMwxPuDKxBuV8qZqNot8bJNgtFKhZKXHdhSw");

/// qsim: small quantum circuits simulated inside a Solana program (solana-qsim), so a program
/// can score a quantum computer's measured counts against the ideal result.
///
/// - `simulate`: one transaction, state on the heap (up to 11 qubits).
/// - `simulate_receipt`: the same, after checking on-chain that the circuit and counts are the
///   ones a qproof receipt records.
/// - `init_state` / `apply` / `apply_range` / `score`: state in an account (up to 20 qubits),
///   circuits spread over many transactions; every applied gate is hash-chained.
#[program]
pub mod qsim {
    use super::*;

    pub fn simulate(_ctx: Context<Simulate>, num_qubits: u8, circuit: Vec<u8>, counts: Vec<Count>) -> Result<QsimScore> {
        instructions::simulate::handle_simulate(num_qubits, circuit, counts)
    }

    pub fn simulate_receipt(ctx: Context<SimulateReceipt>, num_qubits: u8, circuit: Vec<u8>, counts: Vec<Count>) -> Result<QsimScore> {
        instructions::simulate::handle_simulate_receipt(ctx, num_qubits, circuit, counts)
    }

    pub fn init_state(ctx: Context<InitState>, num_qubits: u8) -> Result<()> {
        instructions::state_ops::handle_init_state(ctx, num_qubits)
    }

    pub fn apply(ctx: Context<Apply>, circuit: Vec<u8>) -> Result<()> {
        instructions::state_ops::handle_apply(ctx, circuit)
    }

    pub fn apply_range(ctx: Context<Apply>, gate: Vec<u8>, end: u64) -> Result<()> {
        instructions::state_ops::handle_apply_range(ctx, gate, end)
    }

    pub fn score(ctx: Context<ScoreState>, counts: Vec<Count>) -> Result<QsimScore> {
        instructions::state_ops::handle_score(ctx, counts)
    }
}
