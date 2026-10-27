//! Layout of a qsim state account (owned by this program, created by the client with
//! `system_program::create_account` so it can be up to 10 MiB):
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 8 | magic `QSIMSTv1` |
//! | 8 | 1 | num_qubits |
//! | 9 | 1 | busy (1 = a gate is partly applied by `apply_range`) |
//! | 10 | 1 | pending gate length |
//! | 16 | 8 | gates applied (u64) |
//! | 24 | 32 | circuit hash: h ← sha256(h ‖ gate bytes) for every completed gate |
//! | 56 | 32 | authority |
//! | 88 | 16 | pending gate bytes |
//! | 104 | 8 | next work item of the pending gate (u64) |
//! | 128 | 8·2ⁿ | amplitudes (Q1.30 complex, `solana_qsim::Complex`) |

use anchor_lang::prelude::*;
use solana_qsim::Complex;

use crate::error::ErrorCode;

pub const MAGIC: &[u8; 8] = b"QSIMSTv1";
pub const HEADER_LEN: usize = 128;
pub const MAX_GATE_LEN: usize = 16;

pub const fn state_len(num_qubits: u8) -> usize {
    HEADER_LEN + 8 * (1usize << num_qubits)
}

pub struct Header<'a>(pub &'a mut [u8]);

impl Header<'_> {
    pub fn is_initialized(&self) -> bool {
        &self.0[0..8] == MAGIC
    }
    pub fn num_qubits(&self) -> u8 {
        self.0[8]
    }
    pub fn busy(&self) -> bool {
        self.0[9] == 1
    }
    pub fn gates_applied(&self) -> u64 {
        u64::from_le_bytes(self.0[16..24].try_into().unwrap())
    }
    pub fn circuit_hash(&self) -> [u8; 32] {
        self.0[24..56].try_into().unwrap()
    }
    pub fn authority(&self) -> Pubkey {
        Pubkey::new_from_array(self.0[56..88].try_into().unwrap())
    }
    pub fn pending_gate(&self) -> &[u8] {
        &self.0[88..88 + self.0[10] as usize]
    }
    pub fn next_item(&self) -> u64 {
        u64::from_le_bytes(self.0[104..112].try_into().unwrap())
    }

    pub fn init(&mut self, num_qubits: u8, authority: &Pubkey) {
        self.0[0..8].copy_from_slice(MAGIC);
        self.0[8] = num_qubits;
        self.0[56..88].copy_from_slice(authority.as_ref());
    }
    /// Records a completed gate in the count and the hash chain.
    pub fn complete_gate(&mut self, gate_bytes: &[u8]) {
        let hash = solana_sha256_hasher::hashv(&[&self.circuit_hash(), gate_bytes]).to_bytes();
        self.0[24..56].copy_from_slice(&hash);
        let count = self.gates_applied() + 1;
        self.0[16..24].copy_from_slice(&count.to_le_bytes());
    }
    pub fn set_pending(&mut self, gate_bytes: &[u8], next_item: u64) {
        self.0[9] = 1;
        self.0[10] = gate_bytes.len() as u8;
        self.0[88..88 + gate_bytes.len()].copy_from_slice(gate_bytes);
        self.0[104..112].copy_from_slice(&next_item.to_le_bytes());
    }
    pub fn set_next_item(&mut self, next_item: u64) {
        self.0[104..112].copy_from_slice(&next_item.to_le_bytes());
    }
    pub fn clear_pending(&mut self) {
        self.0[9] = 0;
        self.0[10] = 0;
        self.0[88..112].fill(0);
    }
}

/// Splits account data into the header and the amplitude slice.
pub fn split(data: &mut [u8]) -> Result<(Header<'_>, &mut [Complex])> {
    require!(data.len() >= HEADER_LEN, ErrorCode::StateTooSmall);
    let (header, rest) = data.split_at_mut(HEADER_LEN);
    let n = header[8];
    let len = 1usize << n;
    require!(rest.len() >= 8 * len, ErrorCode::StateTooSmall);
    let bytes = &mut rest[..8 * len];
    require!(bytes.as_ptr() as usize % core::mem::align_of::<Complex>() == 0, ErrorCode::Misaligned);
    // SAFETY: Complex is #[repr(C)] { i32, i32 } (8 bytes, align 4, any bit pattern valid);
    // the slice holds exactly `len` of them and the alignment was checked above.
    let amps = unsafe { core::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut Complex, len) };
    Ok((Header(header), amps))
}
