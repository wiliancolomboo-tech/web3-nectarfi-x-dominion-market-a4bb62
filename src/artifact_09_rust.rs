use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};
use crate::state::{VaultState, TrancheType};
use crate::math::TrancheMath;

#[derive(Accounts)]
pub struct Deposit<'info> {
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

pub fn handler(ctx: Context<Deposit>, amount: u64, tranche: TrancheType) -> Result<()> {
    require_gt!(amount, 0);

    let mut vault = ctx.accounts.vault.load_mut()?;
    let clock = Clock::get()?;

    // 1. Accrue Senior yield up to the present slot
    let elapsed = clock.unix_timestamp.saturating_sub(vault.last_accrual_timestamp);
    let senior_yield = TrancheMath::calculate_senior_accrual(
        vault.total_senior_deposited,
        vault.senior_target_apy_bps,
        elapsed,
    )?;
    
    // Transfer accrued yield from Junior buffer to Senior pool if available
    if senior_yield > 0 && vault.total_junior_deposited >= senior_yield {
        vault.total_senior_deposited = vault.total_senior_deposited.checked_add(senior_yield).unwrap();
        vault.total_junior_deposited = vault.total_junior_deposited.checked_sub(senior_yield).unwrap();
    }
    vault.last_accrual_timestamp = clock.unix_timestamp;

    // 2. Transfer Base Assets to Vault Reserve
    let transfer_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.user_asset_account.to_account_info(),
            to: ctx.accounts.reserve_account.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        },
    );
    token::transfer(transfer_ctx, amount)?;

    // 3. Compute Shares & Mint Tranche Tokens via PDA Signer
    let (shares_to_mint, mint_account) = match tranche {
        TrancheType::Senior => {
            let shares = TrancheMath::calculate_shares_minted(
                amount,
                vault.total_senior_deposited,
                vault.total_senior_shares,
            )?;
            vault.total_senior_deposited = vault.total_senior_deposited.checked_add(amount).unwrap();
            vault.total_senior_shares = vault.total_senior_shares.checked_add(shares).unwrap();
            (shares, ctx.accounts.senior_mint.to_account_info())
        }
        TrancheType::Junior => {
            let shares = TrancheMath::calculate_shares_minted(
                amount,
                vault.total_junior_deposited,
                vault.total_junior_shares,
            )?;
            vault.total_junior_deposited = vault.total_junior_deposited.checked_add(amount).unwrap();
            vault.total_junior_shares = vault.total_junior_shares.checked_add(shares).unwrap();
            (shares, ctx.accounts.junior_mint.to_account_info())
        }
    };

    let asset_mint_key = vault.asset_mint;
    let dominion_pool_key = vault.dominion_pool;
    let bump = vault.bump;

    let signer_seeds: &[&[&[u8]]] = &[&[
        b"vault",
        asset_mint_key.as_ref(),
        dominion_pool_key.as_ref(),
        &[bump],
    ]];

    let mint_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        MintTo {
            mint: mint_account,
            to: ctx.accounts.user_share_account.to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        },
        signer_seeds,
    );
    token::mint_to(mint_ctx, shares_to_mint)?;

    Ok(())
}