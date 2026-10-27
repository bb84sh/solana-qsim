use anchor_lang::prelude::*;

use crate::constants::MAX_BACKEND_LEN;

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub authority: Pubkey,
    pub relayer: Pubkey,
    pub receipt_count: u64,
    pub bump: u8,
}

/// Where a job's results came from. Stored on every receipt so emulator
/// results can never be passed off as real quantum hardware.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Debug)]
pub enum JobSource {
    LocalSimulator,
    CloudEmulator,
    Qpu,
}

#[account]
#[derive(InitSpace)]
pub struct Receipt {
    /// sha256 of the provider's job ID string.
    pub job_id: [u8; 32],
    pub source: JobSource,
    #[max_len(MAX_BACKEND_LEN)]
    pub backend: String,
    /// sha256 of the circuit description that was submitted.
    pub circuit_hash: [u8; 32],
    /// sha256 of the raw measurement results (stored off-chain).
    pub result_hash: [u8; 32],
    pub shots: u32,
    pub relayer: Pubkey,
    pub index: u64,
    pub slot: u64,
    pub timestamp: i64,
    pub bump: u8,
}
