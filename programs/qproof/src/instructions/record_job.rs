use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::ErrorCode,
    state::{Config, JobSource, Receipt},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct RecordJobArgs {
    pub job_id: [u8; 32],
    pub source: JobSource,
    pub backend: String,
    pub circuit_hash: [u8; 32],
    pub result_hash: [u8; 32],
    pub shots: u32,
}

#[derive(Accounts)]
#[instruction(args: RecordJobArgs)]
pub struct RecordJob<'info> {
    #[account(mut)]
    pub relayer: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        init,
        payer = relayer,
        space = 8 + Receipt::INIT_SPACE,
        seeds = [RECEIPT_SEED, args.job_id.as_ref()],
        bump
    )]
    pub receipt: Account<'info, Receipt>,
    pub system_program: Program<'info, System>,
}

pub fn handle_record_job(ctx: Context<RecordJob>, args: RecordJobArgs) -> Result<()> {
    require_keys_eq!(
        ctx.accounts.config.relayer,
        ctx.accounts.relayer.key(),
        ErrorCode::NotRelayer,
    );
    require!(
        !args.backend.is_empty() && args.backend.len() <= MAX_BACKEND_LEN,
        ErrorCode::InvalidBackend,
    );
    require!(args.shots > 0, ErrorCode::ZeroShots);

    let clock = Clock::get()?;
    let config = &mut ctx.accounts.config;
    let receipt = &mut ctx.accounts.receipt;
    receipt.job_id = args.job_id;
    receipt.source = args.source;
    receipt.backend = args.backend;
    receipt.circuit_hash = args.circuit_hash;
    receipt.result_hash = args.result_hash;
    receipt.shots = args.shots;
    receipt.relayer = ctx.accounts.relayer.key();
    receipt.index = config.receipt_count;
    receipt.slot = clock.slot;
    receipt.timestamp = clock.unix_timestamp;
    receipt.bump = ctx.bumps.receipt;
    config.receipt_count += 1;

    msg!(
        "qproof receipt #{} source {:?} backend {} shots {}",
        receipt.index,
        receipt.source,
        receipt.backend,
        receipt.shots
    );
    Ok(())
}
