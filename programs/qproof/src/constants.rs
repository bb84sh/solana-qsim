use anchor_lang::prelude::*;

#[constant]
pub const CONFIG_SEED: &[u8] = b"config";

#[constant]
pub const RECEIPT_SEED: &[u8] = b"receipt";

/// Not exported to the IDL: `#[constant]` does not support usize.
pub const MAX_BACKEND_LEN: usize = 32;
