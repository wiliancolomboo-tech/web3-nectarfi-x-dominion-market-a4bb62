// programs/dominion_nectar_engine/src/lib.rs

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("DNYE111111111111111111111111111111111111111");

#[program]
pub mod dominion_nectar_engine {
    use super::*;

    /// Initializes the global market configuration bridging Dominion and NectarFi
    pub fn initialize_market(
        ctx: Context<InitializeMarket>,
        liquidation_threshold_bps: u64,
        liquidation_bonus_bps: u64,
        min_collateral_ratio_bps: u64,
    ) -> Result<()> {
        require!(
            liquidation_threshold_bps > 1000 && liquidation_threshold_bps <= 10000,
            DNYEError::InvalidRiskParameters
        );
        require!(
            liquidation_bonus_bps > 0 && liquidation_bonus_bps <= 2000,
            DNYEError::InvalidRiskParameters
        );
        require!(
            min_collateral_ratio_bps > liquidation_threshold_bps,
            DNYEError::InvalidRiskParameters
        );

        let config = &mut ctx.accounts.market_config;
        config.admin = ctx.accounts.admin.key();
        config.nectar_vault = ctx.accounts.nectar_vault.key();
        config.nectar_token_mint = ctx.accounts.nectar_token_mint.key();
        config.underlying_mint = ctx.accounts.underlying_mint.key();
        config.dominion_market_id = ctx.accounts.dominion_market_id.key();
        config.liquidation_threshold_bps = liquidation_threshold_bps;
        config.liquidation_bonus_bps = liquidation_bonus_bps;
        config.min_collateral_ratio_bps = min_collateral_ratio_bps;
        config.total_deposited_underlying = 0;
        config.total_nectar_shares = 0;
        config.bump = ctx.bumps.market_config;
        config.vault_bump = ctx.bumps.escrow_vault;

        emit!(MarketInitializedEvent {
            admin: config.admin,
            nectar_vault: config.nectar_vault,
            underlying_mint: config.underlying_mint,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Deposits underlying tokens, mints Nectar yield receipt tokens, and credits Dominion margin
    pub fn deposit_and_stake(ctx: Context<DepositAndStake>, amount: u64) -> Result<()> {
        require!(amount > 0, DNYEError::ZeroAmount);

        // 1. Transfer underlying token from User to DNYE Escrow Vault
        let transfer_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.user_underlying_token.to_account_info(),
                to: ctx.accounts.escrow_vault.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        );
        token::transfer(transfer_ctx, amount)?;

        // 2. Simulated / Real NectarFi Yield Conversion:
        // Exchange Rate calculation: 1 share = (Total Underlying / Total Shares)
        let config = &mut ctx.accounts.market_config;
        let shares_to_mint = if config.total_nectar_shares == 0 || config.total_deposited_underlying == 0 {
            amount
        } else {
            (amount as u128)
                .checked_mul(config.total_nectar_shares as u128)
                .ok_or(DNYEError::MathOverflow)?
                .checked_div(config.total_deposited_underlying as u128)
                .ok_or(DNYEError::MathOverflow)? as u64
        };

        // 3. Update User Margin Account
        let margin_account = &mut ctx.accounts.margin_account;
        if margin_account.owner == Pubkey::default() {
            margin_account.owner = ctx.accounts.user.key();
            margin_account.bump = ctx.bumps.margin_account;
        }

        margin_account.deposited_shares = margin_account
            .deposited_shares
            .checked_add(shares_to_mint)
            .ok_or(DNYEError::MathOverflow)?;
        margin_account.last_update_ts = Clock::get()?.unix_timestamp;

        config.total_deposited_underlying = config
            .total_deposited_underlying
            .checked_add(amount)
            .ok_or(DNYEError::MathOverflow)?;
        config.total_nectar_shares = config
            .total_nectar_shares
            .checked_add(shares_to_mint)
            .ok_or(DNYEError::MathOverflow)?;

        emit!(DepositEvent {
            user: ctx.accounts.user.key(),
            underlying_amount: amount,
            shares_minted: shares_to_mint,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Borrows margin power against Nectar yield shares to trade on Dominion Market
    pub fn borrow_dominion_margin(ctx: Context<BorrowMargin>, borrow_amount: u64) -> Result<()> {
        let config = &ctx.accounts.market_config;
        let margin_account = &mut ctx.accounts.margin_account;

        require!(borrow_amount > 0, DNYEError::ZeroAmount);

        // Convert shares to current underlying value
        let collateral_value = (margin_account.deposited_shares as u128)
            .checked_mul(config.total_deposited_underlying as u128)
            .ok_or(DNYEError::MathOverflow)?
            .checked_div(config.total_nectar_shares as u128)
            .ok_or(DNYEError::MathOverflow)? as u64;

        let new_total_borrow = margin_account
            .borrowed_debt
            .checked_add(borrow_amount)
            .ok_or(DNYEError::MathOverflow)?;

        // Required Collateral = Borrow * (min_collateral_ratio_bps / 10000)
        let required_collateral = (new_total_borrow as u128)
            .checked_mul(config.min_collateral_ratio_bps as u128)
            .ok_or(DNYEError::MathOverflow)?
            .checked_div(10000)
            .ok_or(DNYEError::MathOverflow)? as u64;

        require!(
            collateral_value >= required_collateral,
            DNYEError::InsufficientCollateral
        );

        margin_account.borrowed_debt = new_total_borrow;
        margin_account.last_update_ts = Clock::get()?.unix_timestamp;

        emit!(MarginBorrowedEvent {
            user: margin_account.owner,
            borrow_amount,
            total_debt: new_total_borrow,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Liquidates an undercollateralized Dominion margin position
    pub fn liquidate_position(
        ctx: Context<LiquidatePosition>,
        debt_to_cover: u64,
    ) -> Result<()> {
        let config = &mut ctx.accounts.market_config;
        let margin_account = &mut ctx.accounts.margin_account;

        require!(debt_to_cover > 0, DNYEError::ZeroAmount);
        require!(margin_account.borrowed_debt > 0, DNYEError::NoDebtToLiquidate);

        // Calculate current value of user's shares
        let collateral_value = (margin_account.deposited_shares as u128)
            .checked_mul(config.total_deposited_underlying as u128)
            .ok_or(DNYEError::MathOverflow)?
            .checked_div(config.total_nectar_shares as u128)
            .ok_or(DNYEError::MathOverflow)? as u64;

        // Current Collateral Ratio in BPS: (Collateral Value * 10000) / Borrowed Debt
        let current_ratio_bps = (collateral_value as u128)
            .checked_mul(10000)
            .ok_or(DNYEError::MathOverflow)?
            .checked_div(margin_account.borrowed_debt as u128)
            .ok_or(DNYEError::MathOverflow)? as u64;

        // Verify position is underwater
        require!(
            current_ratio_bps < config.liquidation_threshold_bps,
            DNYEError::PositionNotLiquidatable
        );

        let actual_repay = debt_to_cover.min(margin_account.borrowed_debt);

        // Calculate Collateral to Seize (including bonus)
        // Seize Value = actual_repay * (10000 + bonus_bps) / 10000
        let seize_value = (actual_repay as u128)
            .checked_mul(10000 + config.liquidation_bonus_bps as u128)
            .ok_or(DNYEError::MathOverflow)?
            .checked_div(10000)
            .ok_or(DNYEError::MathOverflow)? as u64;

        // Convert seize_value to shares
        let shares_to_seize = (seize_value as u128)
            .checked_mul(config.total_nectar_shares as u128)
            .ok_or(DNYEError::MathOverflow)?
            .checked_div(config.total_deposited_underlying as u128)
            .ok_or(DNYEError::MathOverflow)? as u64;

        let final_shares_seized = shares_to_seize.min(margin_account.deposited_shares);

        // 1. Liquidator transfers debt payment into Escrow
        let transfer_debt_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.liquidator_token.to_account_info(),
                to: ctx.accounts.escrow_vault.to_account_info(),
                authority: ctx.accounts.liquidator.to_account_info(),
            },
        );
        token::transfer(transfer_debt_ctx, actual_repay)?;

        // 2. Escrow sends seized underlying reward to Liquidator via PDA signature
        let config_seeds = &[
            b"market_config".as_ref(),
            config.underlying_mint.as_ref(),
            &[config.bump],
        ];
        let signer_seeds = &[&config_seeds[..]];

        let transfer_reward_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.escrow_vault.to_account_info(),
                to: ctx.accounts.liquidator_token.to_account_info(),
                authority: ctx.accounts.market_config.to_account_info(),
            },
            signer_seeds,
        );
        token::transfer(transfer_reward_ctx, seize_value.min(config.total_deposited_underlying))?;

        // 3. Update account ledgers
        margin_account.borrowed_debt = margin_account
            .borrowed_debt
            .checked_sub(actual_repay)
            .ok_or(DNYEError::MathOverflow)?;
        margin_account.deposited_shares = margin_account
            .deposited_shares
            .checked_sub(final_shares_seized)
            .ok_or(DNYEError::MathOverflow)?;

        config.total_nectar_shares = config
            .total_nectar_shares
            .checked_sub(final_shares_seized)
            .ok_or(DNYEError::MathOverflow)?;
        config.total_deposited_underlying = config
            .total_deposited_underlying
            .checked_sub(seize_value.min(config.total_deposited_underlying))
            .ok_or(DNYEError::MathOverflow)?;

        emit!(LiquidationExecutedEvent {
            liquidator: ctx.accounts.liquidator.key(),
            borrower: margin_account.owner,
            repaid_debt: actual_repay,
            shares_seized: final_shares_seized,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// ACCOUNT VALIDATION CONTEXTS
// -----------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeMarket<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        init,
        payer = admin,
        space = 8 + MarketConfig::LEN,
        seeds = [b"market_config", underlying_mint.key().as_ref()],
        bump
    )]
    pub market_config: Account<'info, MarketConfig>,

    pub underlying_mint: Account<'info, Mint>,
    pub nectar_token_mint: Account<'info, Mint>,

    /// CHECK: Validated as Dominion Market Router ID
    pub dominion_market_id: UncheckedAccount<'info>,
    /// CHECK: Validated as NectarFi Vault instance
    pub nectar_vault: UncheckedAccount<'info>,

    #[account(
        init,
        payer = admin,
        token::mint = underlying_mint,
        token::authority = market_config,
        seeds = [b"escrow_vault", market_config.key().as_ref()],
        bump
    )]
    pub escrow_vault: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct DepositAndStake<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"market_config", market_config.underlying_mint.as_ref()],
        bump = market_config.bump
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + MarginAccount::LEN,
        seeds = [b"margin_account", market_config.key().as_ref(), user.key().as_ref()],
        bump
    )]
    pub margin_account: Account<'info, MarginAccount>,

    #[account(
        mut,
        constraint = user_underlying_token.owner == user.key(),
        constraint = user_underlying_token.mint == market_config.underlying_mint
    )]
    pub user_underlying_token: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"escrow_vault", market_config.key().as_ref()],
        bump = market_config.vault_bump
    )]
    pub escrow_vault: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct BorrowMargin<'info> {
    pub user: Signer<'info>,

    #[account(
        seeds = [b"market_config", market_config.underlying_mint.as_ref()],
        bump = market_config.bump
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [b"margin_account", market_config.key().as_ref(), user.key().as_ref()],
        bump = margin_account.bump,
        constraint = margin_account.owner == user.key()
    )]
    pub margin_account: Account<'info, MarginAccount>,
}

#[derive(Accounts)]
pub struct LiquidatePosition<'info> {
    #[account(mut)]
    pub liquidator: Signer<'info>,

    #[account(
        mut,
        seeds = [b"market_config", market_config.underlying_mint.as_ref()],
        bump = market_config.bump
    )]
    pub market_config: Account<'info, MarketConfig>,

    #[account(
        mut,
        seeds = [b"margin_account", market_config.key().as_ref(), margin_account.owner.as_ref()],
        bump = margin_account.bump
    )]
    pub margin_account: Account<'info, MarginAccount>,

    #[account(
        mut,
        constraint = liquidator_token.owner == liquidator.key(),
        constraint = liquidator_token.mint == market_config.underlying_mint
    )]
    pub liquidator_token: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"escrow_vault", market_config.key().as_ref()],
        bump = market_config.vault_bump
    )]
    pub escrow_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

// -----------------------------------------------------------------------------
// STATE SCHEMAS & DATA STRUCTURES
// -----------------------------------------------------------------------------

#[account]
pub struct MarketConfig {
    pub admin: Pubkey,
    pub nectar_vault: Pubkey,
    pub nectar_token_mint: Pubkey,
    pub underlying_mint: Pubkey,
    pub dominion_market_id: Pubkey,
    pub liquidation_threshold_bps: u64, // e.g., 11500 = 115%
    pub liquidation_bonus_bps: u64,     // e.g., 500 = 5%
    pub min_collateral_ratio_bps: u64,  // e.g., 13000 = 130%
    pub total_deposited_underlying: u64,
    pub total_nectar_shares: u64,
    pub bump: u8,
    pub vault_bump: u8,
}

impl MarketConfig {
    pub const LEN: usize = 32 + 32 + 32 + 32 + 32 + 8 + 8 + 8 + 8 + 8 + 1 + 1;
}

#[account]
pub struct MarginAccount {
    pub owner: Pubkey,
    pub deposited_shares: u64,
    pub borrowed_debt: u64,
    pub last_update_ts: i64,
    pub bump: u8,
}

impl MarginAccount {
    pub const LEN: usize = 32 + 8 + 8 + 8 + 1;
}

// -----------------------------------------------------------------------------
// EVENTS & ERRORS
// -----------------------------------------------------------------------------

#[event]
pub struct MarketInitializedEvent {
    pub admin: Pubkey,
    pub nectar_vault: Pubkey,
    pub underlying_mint: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct DepositEvent {
    pub user: Pubkey,
    pub underlying_amount: u64,
    pub shares_minted: u64,
    pub timestamp: i64,
}

#[event]
pub struct MarginBorrowedEvent {
    pub user: Pubkey,
    pub borrow_amount: u64,
    pub total_debt: u64,
    pub timestamp: i64,
}

#[event]
pub struct LiquidationExecutedEvent {
    pub liquidator: Pubkey,
    pub borrower: Pubkey,
    pub repaid_debt: u64,
    pub shares_seized: u64,
    pub timestamp: i64,
}

#[error_code]
pub enum DNYEError {
    #[msg("Supplied risk configuration values are out of allowable bounds")]
    InvalidRiskParameters,
    #[msg("Operation amount must be strictly greater than zero")]
    ZeroAmount,
    #[msg("Math calculation resulted in arithmetic overflow")]
    MathOverflow,
    #[msg("Insufficient collateral to sustain requested borrow")]
    InsufficientCollateral,
    #[msg("Position does not possess any outstanding debt to liquidate")]
    NoDebtToLiquidate,
    #[msg("Collateralization health is above liquidation threshold")]
    PositionNotLiquidatable,
}