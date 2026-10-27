use anchor_lang::prelude::*;

use crate::{constants::*, error::ErrorCode, state::Config};

#[derive(Accounts)]
pub struct SetRelayer<'info> {
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    pub authority: Signer<'info>,
}

pub fn handle_set_relayer(ctx: Context<SetRelayer>, relayer: Pubkey) -> Result<()> {
    require_keys_eq!(
        ctx.accounts.config.authority,
        ctx.accounts.authority.key(),
        ErrorCode::Unauthorized,
    );
    ctx.accounts.config.relayer = relayer;
    msg!("qproof relayer set to {}", relayer);
    Ok(())
}
