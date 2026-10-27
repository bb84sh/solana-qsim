pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("6pxqWGoxSRBwmL6GJie9BD43HGe3ybFu9xmJQ24TvcXB");

#[program]
pub mod qproof {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, relayer: Pubkey) -> Result<()> {
        crate::instructions::initialize::handle_initialize(ctx, relayer)
    }

    pub fn set_relayer(ctx: Context<SetRelayer>, relayer: Pubkey) -> Result<()> {
        crate::instructions::set_relayer::handle_set_relayer(ctx, relayer)
    }

    pub fn record_job(ctx: Context<RecordJob>, args: RecordJobArgs) -> Result<()> {
        crate::instructions::record_job::handle_record_job(ctx, args)
    }
}
