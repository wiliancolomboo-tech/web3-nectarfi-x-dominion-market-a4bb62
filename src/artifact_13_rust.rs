use anchor_lang::prelude::*;

pub mod errors;
pub mod instructions;
pub mod math;
pub mod state;

use instructions::*;

declare_id!("NectDom111111111111111111111111111111111111");

#[program]
pub mod nectar_dominion {
    use super::*;

    pub fn initialize_market(ctx: Context<InitializeMarket>, market_id: [u8; 32]) -> Result<()> {
        instructions::initialize_market::handler(ctx, market_id)
    }

    pub fn split_collateral(ctx: Context<SplitCollateral>, amount: u64) -> Result<()> {
        instructions::split_collateral::handler(ctx, amount)
    }

    pub fn harvest_yield(ctx: Context<HarvestYield>, current_index: u128) -> Result<()> {
        instructions::harvest_yield::handler(ctx, current_index)
    }

    pub fn resolve_market(ctx: Context<ResolveMarket>, winning_outcome: u8) -> Result<()> {
        instructions::resolve_market::handler(ctx, winning_outcome)
    }

    pub fn settle_redeem(ctx: Context<SettleAndRedeem>, amount: u64) -> Result<()> {
        instructions::settle_redeem::handler(ctx, amount)
    }
}