use anchor_lang::prelude::*;
use solana_qsim::{Complex, Gate, State};

use crate::{error::ErrorCode, instructions::*};

/// Largest register `simulate` holds on the 32 KiB heap (2^11 × 8 bytes = 16 KiB).
pub const MAX_HEAP_QUBITS: u8 = 11;

/// Stateless: anyone can call it; nothing is stored.
#[derive(Accounts)]
pub struct Simulate {}

/// `simulate`, but only for counts a qproof receipt vouches for.
#[derive(Accounts)]
pub struct SimulateReceipt<'info> {
    /// The QPU job's receipt (owner checked by Anchor: qproof).
    pub receipt: Account<'info, qproof::state::Receipt>,
}

/// Runs `circuit` from |0…0⟩ and scores `counts` against the result, in one transaction.
pub fn handle_simulate(num_qubits: u8, circuit: Vec<u8>, counts: Vec<Count>) -> Result<QsimScore> {
    let (amps, hash) = run_circuit(num_qubits, &circuit)?;
    let score = solana_qsim::score_amplitudes(&amps, &to_counts(&counts)).map_err(ErrorCode::from)?;
    Ok(report(num_qubits, score, hash))
}

/// Like `simulate`, but first checks, on-chain, that the circuit and the counts are exactly the
/// ones the receipt records: the circuit hash chain must equal `receipt.circuit_hash`, and
/// sha256 of the canonical counts (sorted "bits:count" lines, as tools/sample.py writes them)
/// must equal `receipt.result_hash`, with the same number of shots.
pub fn handle_simulate_receipt(ctx: Context<SimulateReceipt>, num_qubits: u8, circuit: Vec<u8>, counts: Vec<Count>) -> Result<QsimScore> {
    let receipt = &ctx.accounts.receipt;
    let (amps, hash) = run_circuit(num_qubits, &circuit)?;
    require!(hash == receipt.circuit_hash, ErrorCode::CircuitMismatch);
    let shots: u64 = counts.iter().map(|c| c.count as u64).sum();
    require!(shots == receipt.shots as u64, ErrorCode::ResultMismatch);
    require!(counts_hash(num_qubits, &counts) == receipt.result_hash, ErrorCode::ResultMismatch);
    let score = solana_qsim::score_amplitudes(&amps, &to_counts(&counts)).map_err(ErrorCode::from)?;
    msg!("qsim: counts match qproof receipt #{} ({:?}, {})", receipt.index, receipt.source, receipt.backend);
    Ok(report(num_qubits, score, hash))
}

/// sha256 over "bits:count" lines joined by "\n", bits most significant qubit first. Outcomes
/// sorted by value are also sorted as fixed-width bitstrings.
pub fn counts_hash(num_qubits: u8, counts: &[Count]) -> [u8; 32] {
    let mut text = Vec::with_capacity(counts.len() * (num_qubits as usize + 12));
    for (i, c) in counts.iter().enumerate() {
        if i > 0 {
            text.push(b'\n');
        }
        for q in (0..num_qubits).rev() {
            text.push(if c.outcome >> q & 1 == 1 { b'1' } else { b'0' });
        }
        text.push(b':');
        let mut digits = [0u8; 10];
        let mut n = c.count;
        let mut k = digits.len();
        loop {
            k -= 1;
            digits[k] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        text.extend_from_slice(&digits[k..]);
    }
    solana_sha256_hasher::hashv(&[&text]).to_bytes()
}

/// Runs a circuit on the heap from |0…0⟩; returns the amplitudes and the circuit hash chain.
fn run_circuit(num_qubits: u8, circuit: &[u8]) -> Result<(Vec<Complex>, [u8; 32])> {
    require!(num_qubits >= 1 && num_qubits <= MAX_HEAP_QUBITS, ErrorCode::TooManyQubits);
    let mut amps = vec![Complex::ZERO; 1 << num_qubits];
    amps[0] = Complex::ONE;
    let mut state = State::new(&mut amps).map_err(ErrorCode::from)?;
    // Same hash chain as a state account starting from zero, so both paths name a circuit alike.
    let mut hash = [0u8; 32];
    let mut rest = circuit;
    while !rest.is_empty() {
        let (gate, used) = Gate::decode(rest).map_err(ErrorCode::from)?;
        state.apply(&gate).map_err(ErrorCode::from)?;
        hash = solana_sha256_hasher::hashv(&[&hash, &rest[..used]]).to_bytes();
        rest = &rest[used..];
    }
    Ok((amps, hash))
}
