use anchor_lang::prelude::*;
use solana_qsim::{Complex, Gate, State, MAX_QUBITS};

use crate::{error::ErrorCode, instructions::*, state::*};

#[derive(Accounts)]
pub struct InitState<'info> {
    pub authority: Signer<'info>,
    /// CHECK: a fresh account the client created with this program as owner; layout in state.rs.
    #[account(mut, owner = crate::ID)]
    pub state: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct Apply<'info> {
    pub authority: Signer<'info>,
    /// CHECK: an initialized qsim state (checked in the handler); layout in state.rs.
    #[account(mut, owner = crate::ID)]
    pub state: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct ScoreState<'info> {
    /// CHECK: an initialized qsim state (checked in the handler); read only.
    #[account(owner = crate::ID)]
    pub state: UncheckedAccount<'info>,
}

/// Sets the state to |0…0⟩. Only this program writes its accounts, and it writes the magic
/// first, so an account without the magic still holds the zeros it was created with.
pub fn handle_init_state(ctx: Context<InitState>, num_qubits: u8) -> Result<()> {
    require!(num_qubits >= 1 && num_qubits as u32 <= MAX_QUBITS, ErrorCode::TooManyQubits);
    let mut data = ctx.accounts.state.try_borrow_mut_data()?;
    require!(data.len() >= state_len(num_qubits), ErrorCode::StateTooSmall);
    require!(data[..HEADER_LEN].iter().all(|b| *b == 0), ErrorCode::AlreadyInitialized);
    Header(&mut data[..HEADER_LEN]).init(num_qubits, &ctx.accounts.authority.key());
    let (_, amps) = split(&mut data)?;
    amps[0] = Complex::ONE;
    msg!("qsim state: {} qubits", num_qubits);
    Ok(())
}

fn writable<'a>(data: &'a mut [u8], authority: &Pubkey) -> Result<(Header<'a>, &'a mut [Complex])> {
    let (header, amps) = split(data)?;
    require!(header.is_initialized(), ErrorCode::NotInitialized);
    require_keys_eq!(header.authority(), *authority, ErrorCode::Unauthorized);
    Ok((header, amps))
}

/// Applies whole gates; each is added to the hash chain.
pub fn handle_apply(ctx: Context<Apply>, circuit: Vec<u8>) -> Result<()> {
    let mut data = ctx.accounts.state.try_borrow_mut_data()?;
    let (mut header, amps) = writable(&mut data, &ctx.accounts.authority.key())?;
    require!(!header.busy(), ErrorCode::GateInProgress);
    let mut state = State::new(amps).map_err(ErrorCode::from)?;
    let mut rest = circuit.as_slice();
    while !rest.is_empty() {
        let (gate, used) = Gate::decode(rest).map_err(ErrorCode::from)?;
        state.apply(&gate).map_err(ErrorCode::from)?;
        header.complete_gate(&rest[..used]);
        rest = &rest[used..];
    }
    msg!("qsim: {} gates applied", header.gates_applied());
    Ok(())
}

/// Applies one gate to work items [next, end). The first call starts the gate at item 0; later
/// calls must pass the same gate bytes and continue where the last one stopped. The gate joins
/// the hash chain when its last item is done.
pub fn handle_apply_range(ctx: Context<Apply>, gate_bytes: Vec<u8>, end: u64) -> Result<()> {
    require!(gate_bytes.len() <= MAX_GATE_LEN, ErrorCode::InvalidCircuit);
    let (gate, used) = Gate::decode(&gate_bytes).map_err(ErrorCode::from)?;
    require!(used == gate_bytes.len(), ErrorCode::InvalidCircuit);

    let mut data = ctx.accounts.state.try_borrow_mut_data()?;
    let (mut header, amps) = writable(&mut data, &ctx.accounts.authority.key())?;
    let start = if header.busy() {
        require!(header.pending_gate() == gate_bytes.as_slice(), ErrorCode::BadRange);
        header.next_item()
    } else {
        0
    };
    let mut state = State::new(amps).map_err(ErrorCode::from)?;
    let items = state.work_items(&gate) as u64;
    require!(end > start && end <= items, ErrorCode::BadRange);
    state.apply_range(&gate, start as usize, end as usize).map_err(ErrorCode::from)?;

    if end == items {
        header.clear_pending();
        header.complete_gate(&gate_bytes);
    } else if header.busy() {
        header.set_next_item(end);
    } else {
        header.set_pending(&gate_bytes, end);
    }
    Ok(())
}

/// Scores counts against the state; anyone can call it.
pub fn handle_score(ctx: Context<ScoreState>, counts: Vec<Count>) -> Result<QsimScore> {
    let data = ctx.accounts.state.try_borrow_data()?;
    require!(data.len() >= HEADER_LEN && &data[..8] == MAGIC, ErrorCode::NotInitialized);
    require!(data[9] == 0, ErrorCode::GateInProgress);
    let num_qubits = data[8];
    let bytes = data.get(HEADER_LEN..state_len(num_qubits)).ok_or(ErrorCode::StateTooSmall)?;
    require!(bytes.as_ptr() as usize % core::mem::align_of::<Complex>() == 0, ErrorCode::Misaligned);
    // SAFETY: see state::split; read-only view of the same layout.
    let amps = unsafe { core::slice::from_raw_parts(bytes.as_ptr() as *const Complex, 1usize << num_qubits) };
    let score = solana_qsim::score_amplitudes(amps, &to_counts(&counts)).map_err(ErrorCode::from)?;
    let hash: [u8; 32] = data[24..56].try_into().unwrap();
    Ok(report(num_qubits, score, hash))
}
