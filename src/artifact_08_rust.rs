use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};
use crate::state::{VaultState, InitializeVaultParams};

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    /// CHECK: Dominion market isolated pool address validated via governance
    pub dominion_pool: AccountInfo<'info>,

    /// CHECK: Low-latency Pyth Price Account
    pub pyth_oracle_feed: AccountInfo<'info>,

    #[account(
        init,
        payer = authority,
        space = VaultState::LEN,
        seeds = [b"vault", asset_mint.key().as_ref(), dominion_pool.key().as_ref()],
        bump
    )]
    pub vault: AccountLoader<'info, VaultState>,

    #[account(
        init,
        payer = authority,
        token::mint = asset_mint,
        token::authority = vault,
        seeds = [b"reserve", vault.key().as_ref()],
        bump
    )]
    pub reserve_account: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = authority,
        seeds = [b"mint_senior", vault.key().as_ref()],
        bump,
        mint::decimals = asset_mint.decimals,
        mint::authority = vault
    )]
    pub senior_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = authority,
        seeds = [b"mint_junior", vault.key().as_ref()],
        bump,
        mint::decimals = asset_mint.decimals,
        mint::authority = vault
    )]
    pub junior_mint: Account<'info, Mint>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handler(ctx: Context<InitializeVault>, params: InitializeVaultParams) -> Result<()> {
    let mut vault = ctx.accounts.vault.load_init()?;
    vault.authority = ctx.accounts.authority.key();
    vault.asset_mint = ctx.accounts.asset_mint.key();
    vault.dominion_pool = ctx.accounts.dominion_pool.key();
    vault.reserve_account = ctx.accounts.reserve_account.key();
    vault.senior_mint = ctx.accounts.senior_mint.key();
    vault.junior_mint = ctx.accounts.junior_mint.key();
    vault.pyth_oracle_feed = ctx.accounts.pyth_oracle_feed.key();

    vault.total_senior_deposited = 0;
    vault.total_junior_deposited = 0;
    vault.total_senior_shares = 0;
    vault.total_junior_shares = 0;
    vault.dominion_active_debt = 0;

    vault.senior_target_apy_bps = params.senior_target_apy_bps;
    vault.target_utilization_bps = params.target_utilization_bps;
    vault.max_junior_leverage_bps = params.max_junior_leverage_bps;
    vault.last_accrual_timestamp = Clock::get()?.unix_timestamp;
    vault.bump = ctx.bumps.vault;

    Ok(())
}