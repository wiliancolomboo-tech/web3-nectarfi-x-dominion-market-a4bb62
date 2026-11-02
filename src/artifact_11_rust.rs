use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount, Transfer};
use crate::state::{VaultState, TrancheType};
use crate::math::TrancheMath;
use crate::errors::ErrorCode;

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(mut)]
    pub vault: AccountLoader<'info, VaultState>,

    #[account(mut)]
    pub user_asset_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub reserve_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub senior_mint: Account<'info, Mint>,

    #[account(mut)]
    pub junior_mint: Account<'info, Mint>,

    #[account(mut)]
    pub user_share_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<Withdraw>, shares_to_burn: u64, tranche: TrancheType) -> Result<()> {
    require_gt!(shares_to_burn, 0);

    let mut vault = ctx.accounts.vault.load_mut()?;

    let (assets_to_return, mint_account) = match tranche {
        TrancheType::Senior => {
            let assets = TrancheMath::calculate_assets_redeemed(
                shares_to_burn,
                vault.total_senior_deposited,
                vault.total_senior_shares,
            )?;
            vault.total_senior_deposited = vault.total_senior_deposited.checked_sub(assets).ok_or(ErrorCode::MathOverflow)?;
            vault.total_senior_shares = vault.total_senior_shares.checked_sub(shares_to_burn).ok_or(ErrorCode::MathOverflow)?;
            (assets, ctx.accounts.senior_mint.to_account_info())
        }
        TrancheType::Junior => {
            let assets = TrancheMath::calculate_assets_redeemed(
                shares_to_burn,
                vault.total_junior_deposited,
                vault.total_junior_shares,
            )?;
            vault.total_junior_deposited = vault.total_junior_deposited.checked_sub(assets).ok_or(ErrorCode::MathOverflow)?;
            vault.total_junior_shares = vault.total_junior_shares.checked_sub(shares_to_burn).ok_or(ErrorCode::MathOverflow)?;
            (assets, ctx.accounts.junior_mint.to_account_info())
        }
    };

    require_gte!(ctx.accounts.reserve_account.amount, assets_to_return, ErrorCode::InsufficientVaultLiquidity);

    // 1. Burn Tranche Shares
    let burn_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        Burn {
            mint: mint_account,
            from: ctx.accounts.user_share_account.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        },
    );
    token::burn(burn_ctx, shares_to_burn)?;

    // 2. Transfer Base Assets back to User from Reserve PDA
    let asset_mint_key = vault.asset_mint;
    let dominion_pool_key = vault.dominion_pool;
    let bump = vault.bump;

    let signer_seeds: &[&[&[u8]]] = &[&[
        b"vault",
        asset_mint_key.as_ref(),
        dominion_pool_key.as_ref(),
        &[bump],
    ]];

    let transfer_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.reserve_account.to_account_info(),
            to: ctx.accounts.user_asset_account.to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        },
        signer_seeds,
    );
    token::transfer(transfer_ctx, assets_to_return)?;

    Ok(())
}