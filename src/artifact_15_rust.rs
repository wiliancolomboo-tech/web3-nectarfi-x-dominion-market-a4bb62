use anchor_lang::prelude::*;

pub mod errors;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("NecDom1111111111111111111111111111111111111");

#[program]
pub mod nectar_dominion {
    use super::*;

    pub fn initialize_market(
        ctx: Context<InitializeMarket>,
        market_id: u64,
        settlement_timestamp: i64,
        fee_bps: u16,
        liquidation_penalty_bps: u16,
        min_collat_ratio_bps: u16,
    ) -> Result<()> {
        instructions::initialize::handler(
            ctx,
            market_id,
            settlement_timestamp,
            fee_bps,
            liquidation_penalty_bps,
            min_collat_ratio_bps,
        )
    }

    pub fn deposit_collateral(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
        instructions::deposit_collateral::handler(ctx, amount)
    }

    pub fn mint_structured_claims(ctx: Context<MintStructuredClaims>, claim_amount: u64) -> Result<()> {
        instructions::mint_claims::handler(ctx, claim_amount)
    }

    pub fn accrue_yield_and_rebalance(ctx: Context<AccrueYieldAndRebalance>, yield_delta_bps: u16) -> Result<()> {
        instructions::accrue_yield::handler(ctx, yield_delta_bps)
    }

    pub fn liquidate_undercollateralized(ctx: Context<LiquidateUndercollateralized>) -> Result<()> {
        instructions::liquidate::handler(ctx)
    }

    pub fn settle_market(ctx: Context<SettleMarket>, settlement_price: u64, senior_payout_ratio_bps: u16) -> Result<()> {
        instructions::settle::handler(ctx, settlement_price, senior_payout_ratio_bps)
    }

    pub fn redeem_claims(ctx: Context<RedeemClaims>, senior_amount: u64, junior_amount: u64) -> Result<()> {
        instructions::redeem::handler(ctx, senior_amount, junior_amount)
    }
}