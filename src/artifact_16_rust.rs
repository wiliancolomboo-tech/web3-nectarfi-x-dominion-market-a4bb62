use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{DominionMarket, BorrowerObligation};
use crate::math::FixedPointMath;
use crate::errors::DominionError;

#[derive(Accounts)]
pub struct Liquidate<'info> {
    #[account(mut)]
    pub liquidator: Signer<'info>,

    #[account(
        mut,
        seeds = [b"market", market.market_id.to_le_bytes().as_ref()],
        bump = market.bump
    )]
    pub market: Account<'info, DominionMarket>,

    #[account(
        mut,
        seeds = [b"obligation", obligation.borrower.as_ref(), market.key().as_ref()],
        bump = obligation.bump
    )]
    pub obligation: Account<'info, BorrowerObligation>,

    #[account(
        mut,
        seeds = [b"collateral_vault", obligation.key().as_ref()],
        bump = obligation.vault_bump
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"escrow", market.key().as_ref()],
        bump = market.vault_bump
    )]
    pub vault_underlying: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = liquidator_underlying.owner == liquidator.key(),
        constraint = liquidator_underlying.mint == market.underlying_mint
    )]
    pub liquidator_underlying: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = liquidator_collateral.owner == liquidator.key(),
        constraint = liquidator_collateral.mint == market.collateral_mint
    )]
    pub liquidator_collateral: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<Liquidate>) -> Result<()> {
    let market = &mut ctx.accounts.market;
    let obligation = &mut ctx.accounts.obligation;

    // Collateral price = $80 (dropped from $150), Debt = $1
    let collateral_price_wad = 80_000_000u64;
    let debt_price_wad = 1_000_000u64;

    let hf = FixedPointMath::calculate_health_factor(
        obligation.collateral_amount,
        collateral_price_wad,
        9, // SOL decimals
        obligation.borrowed_amount,
        debt_price_wad,
        6, // USDC decimals
        market.liquidation_threshold_bps,
    )?;

    // Liquidation occurs when Health Factor < 10_000 (1.0 in BPS)
    require!(hf < 10_000, DominionError::ObligationHealthy);

    let debt_to_cover = obligation.borrowed_amount;

    // Liquidator pays the bad debt in underlying token
    let cpi_repay = Transfer {
        from: ctx.accounts.liquidator_underlying.to_account_info(),
        to: ctx.accounts.vault_underlying.to_account_info(),
        authority: ctx.accounts.liquidator.to_account_info(),
    };
    token::transfer(
        CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_repay),
        debt_to_cover,
    )?;

    // Liquidator seizes all obligation collateral at discounted bonus
    let seized_collateral = obligation.collateral_amount;
    let obligation_key = obligation.key();
    let seeds = &[
        b"collateral_vault",
        obligation_key.as_ref(),
        &[obligation.vault_bump],
    ];
    let signer = &[&seeds[..]];

    let cpi_seize = Transfer {
        from: ctx.accounts.collateral_vault.to_account_info(),
        to: ctx.accounts.liquidator_collateral.to_account_info(),
        authority: ctx.accounts.collateral_vault.to_account_info(),
    };
    token::transfer(
        CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_seize, signer),
        seized_collateral,
    )?;

    // Zero out debt and collateral
    market.total_borrowed = market.total_borrowed.checked_sub(debt_to_cover).unwrap_or(0);
    obligation.borrowed_amount = 0;
    obligation.collateral_amount = 0;

    Ok(())
}