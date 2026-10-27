use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Circuit bytes are not valid (unknown gate, truncated, or a qubit outside the register)")]
    InvalidCircuit,
    #[msg("Too many qubits for this instruction")]
    TooManyQubits,
    #[msg("Counts must be non-empty, strictly increasing and inside the register")]
    BadCounts,
    #[msg("State account is too small for this many qubits")]
    StateTooSmall,
    #[msg("State account is already initialized")]
    AlreadyInitialized,
    #[msg("State account is not an initialized qsim state")]
    NotInitialized,
    #[msg("Only the state's authority can change it")]
    Unauthorized,
    #[msg("A gate is partly applied; finish it with apply_range first")]
    GateInProgress,
    #[msg("apply_range must continue the pending gate, in order, inside its work items")]
    BadRange,
    #[msg("State account data is not aligned")]
    Misaligned,
    #[msg("The circuit is not the one the qproof receipt records")]
    CircuitMismatch,
    #[msg("The counts are not the results the qproof receipt records")]
    ResultMismatch,
}

impl From<solana_qsim::Error> for ErrorCode {
    fn from(e: solana_qsim::Error) -> Self {
        match e {
            solana_qsim::Error::BadCounts => ErrorCode::BadCounts,
            solana_qsim::Error::BadRange => ErrorCode::BadRange,
            solana_qsim::Error::BadStateLength => ErrorCode::TooManyQubits,
            _ => ErrorCode::InvalidCircuit,
        }
    }
}
