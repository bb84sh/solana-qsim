use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Only the config authority can do this")]
    Unauthorized,
    #[msg("Only the registered relayer can record jobs")]
    NotRelayer,
    #[msg("Backend name is empty or longer than 32 bytes")]
    InvalidBackend,
    #[msg("Shots must be greater than zero")]
    ZeroShots,
}
