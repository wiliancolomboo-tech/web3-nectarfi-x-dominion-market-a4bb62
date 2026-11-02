use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use pyth_sdk_solana::load_price_feed_from_account_info;
use crate::state::VaultState;
use crate::errors::ErrorCode;

#[derive(Accounts)]
pub struct Rebalance<'info> {
    #[account(mut)]
    pub keeper: Signer<'info>,

    #[account(mut)]
    pub vault: AccountLoader<'info, VaultState>,

    #[account(mut)]
    pub reserve_account: Account<'info, TokenAccount>,

    /// CHECK: Dominion Market Isolated Borrow / Supply Reserve
    #[account(mut)]
    pub dominion_reserve: AccountInfo<'info>,

    /// CHECK: Pyth Oracle VAA feed account
    pub pyth_oracle: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<Rebalance>) -> Result<()> {
    let mut vault = ctx.accounts.vault.load_mut()?;

    // 1. Pyth Oracle Price & Confidence Check
    let price_feed = load_price_feed_from_account_info(&ctx.accounts.pyth_oracle)
        .map_err(|_| ErrorCode::InvalidOracleFeed)?;
    
    let clock = Clock::get()?;
    // Maximum 30 seconds price staleness allowed
    let current_price = price_feed
        .get_price_no_older_than(clock.unix_timestamp, 30)
        .ok_or(ErrorCode::InvalidOracleFeed)?;

    // Guard: Reject if confidence interval is > 2% of price
    let max_conf_threshold = (current_price.price.unsigned_abs() as u64) / 50;
    require_gte!(max_conf_threshold, current_price.conf, ErrorCode::OracleConfidenceTooWide);

    // 2. Liquidity Rebalancing Math
    let idle_liquidity = ctx.accounts.reserve_account.amount;
    let total_capital = vault.total_senior_deposited + vault.total_junior_deposited;

    let target_deployment = (total_capital as u128 * vault.target_utilization_bps as u128 / 10000) as u64;

    if vault.dominion_active_debt < target_deployment && idle_liquidity > 0 {
        // Deploy capital into Dominion pool
        let deployable_amount = std::cmp::min(idle_liquidity, target_deployment - vault.dominion_active_debt);

        let asset_key = vault.asset_mint;
        let dominion_key = vault.dominion_pool;
        let bump = vault.bump;

        let seeds: &[&[&[u8]]] = &[&[
            b"vault",
            asset_key.as_ref(),
            dominion_key.as_ref(),
            &[bump],
        ]];

        let cpi_transfer = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.reserve_account.to_account_info(),
                to: ctx.accounts.dominion_reserve.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            seeds,
        );
        token::transfer(cpi_transfer, deployable_amount)?;

        vault.dominion_active_debt = vault.dominion_active_debt.checked_add(deployable_amount).unwrap();
    }

    Ok(())
}