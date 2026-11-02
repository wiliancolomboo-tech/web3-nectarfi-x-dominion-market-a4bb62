use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("NectDominionMarketVault1111111111111111111111");

pub const BPS_DIVISOR: u64 = 10_000;
pub const VIRTUAL_INDEX_PRECISION: u128 = 1_000_000_000_000; // 1e12 fixed point

#[program]
pub mod nectar_dominion_vault {
    use super::*;

    /// Initializes a new Dominion Market outcome pool tied to a NectarFi Yield Strategy.
    pub fn initialize_market(
        ctx: Context<InitializeMarket>,
        market_id: String,
        settlement_timestamp: i64,
        protocol_fee_bps: u64,
    ) -> Result<()> {
        require!(
            settlement_timestamp > Clock::get()?.unix_timestamp,
            VaultError::InvalidSettlementTime
        );
        require!(protocol_fee_bps <= 1_000, VaultError::ExcessiveFeeBps);
        require!(market_id.len() <= 32, VaultError::MarketIdTooLong);

        let market = &mut ctx.accounts.market_account;
        market.authority = ctx.accounts.authority.key();
        market.oracle = ctx.accounts.oracle.key();
        market.market_id = market_id;
        market.underlying_mint = ctx.accounts.underlying_mint.key();
        market.nectar_receipt_mint = ctx.accounts.nectar_receipt_mint.key();
        market.yes_token_mint = ctx.accounts.yes_token_mint.key();
        market.no_token_mint = ctx.accounts.no_token_mint.key();
        market.vault_bump = ctx.bumps.vault_escrow;
        market.settlement_timestamp = settlement_timestamp;
        market.protocol_fee_bps = protocol_fee_bps;
        market.is_resolved = false;
        market.winning_outcome = Outcome::Unresolved;
        market.total_underwritten_amount = 0;
        market.total_nectar_shares_locked = 0;
        market.settled_yield_per_share = 0;
        market.initial_yield_index = VIRTUAL_INDEX_PRECISION; // Baseline 1.0 index

        emit!(MarketInitializedEvent {
            market: market.key(),
            market_id: market.market_id.clone(),
            authority: market.authority,
            settlement_timestamp,
        });

        Ok(())
    }

    /// Deposits underlying collateral ($USDC) into NectarFi's yield pool, receives Nectar receipt
    /// shares, and atomically mints paired Dominion YES and NO conditional tokens.
    pub fn deposit_and_underwrite(
        ctx: Context<DepositAndUnderwrite>,
        underlying_amount: u64,
    ) -> Result<()> {
        require!(underlying_amount > 0, VaultError::ZeroDepositAmount);
        let market = &mut ctx.accounts.market_account;
        require!(!market.is_resolved, VaultError::MarketAlreadyResolved);
        require!(
            Clock::get()?.unix_timestamp < market.settlement_timestamp,
            VaultError::TradingWindowClosed
        );

        // 1. Transfer underlying assets from User to NectarFi Escrow Vault
        let transfer_cpi = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.user_underlying.to_account_info(),
                to: ctx.accounts.nectar_reserve_vault.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        );
        token::transfer(transfer_cpi, underlying_amount)?;

        // 2. Simulating NectarFi yield accrual / shares ratio:
        // In real execution, nectar_reserve_vault issues receipt shares at current index.
        // For standard liquidity parity, 1:1 shares credited at init, scaling with index.
        let receipt_shares = underlying_amount;

        // Transfer receipt shares from Nectar reserve into our programmatic Vault Escrow
        let receipt_transfer_cpi = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.nectar_pool_receipt_source.to_account_info(),
                to: ctx.accounts.vault_escrow.to_account_info(),
                authority: ctx.accounts.nectar_pool_authority.to_account_info(),
            },
        );
        token::transfer(receipt_transfer_cpi, receipt_shares)?;

        // 3. Mint paired Dominion conditional tokens (YES and NO) to user
        let seeds = &[
            b"market_config".as_ref(),
            market.market_id.as_bytes(),
            &[market.vault_bump],
        ];
        let signer = &[&seeds[..]];

        // Mint YES tokens
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::MintTo {
                    mint: ctx.accounts.yes_token_mint.to_account_info(),
                    to: ctx.accounts.user_yes_account.to_account_info(),
                    authority: ctx.accounts.vault_escrow.to_account_info(),
                },
                signer,
            ),
            underlying_amount,
        )?;

        // Mint NO tokens
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::MintTo {
                    mint: ctx.accounts.no_token_mint.to_account_info(),
                    to: ctx.accounts.user_no_account.to_account_info(),
                    authority: ctx.accounts.vault_escrow.to_account_info(),
                },
                signer,
            ),
            underlying_amount,
        )?;

        market.total_underwritten_amount = market
            .total_underwritten_amount
            .checked_add(underlying_amount)
            .ok_or(VaultError::MathOverflow)?;

        market.total_nectar_shares_locked = market
            .total_nectar_shares_locked
            .checked_add(receipt_shares)
            .ok_or(VaultError::MathOverflow)?;

        emit!(UnderwriteDepositEvent {
            market: market.key(),
            user: ctx.accounts.user.key(),
            amount: underlying_amount,
            shares: receipt_shares,
        });

        Ok(())
    }

    /// Oracle/Authority triggers resolution based on Dominion real-world outcome
    pub fn resolve_market(
        ctx: Context<ResolveMarket>,
        outcome: Outcome,
        final_yield_index: u128,
    ) -> Result<()> {
        let market = &mut ctx.accounts.market_account;
        require!(!market.is_resolved, VaultError::MarketAlreadyResolved);
        require!(
            outcome == Outcome::Yes || outcome == Outcome::No,
            VaultError::InvalidOutcome
        );
        require!(
            final_yield_index >= market.initial_yield_index,
            VaultError::YieldIndexRegressed
        );

        market.is_resolved = true;
        market.winning_outcome = outcome;
        market.settled_yield_per_share = final_yield_index;

        emit!(MarketResolvedEvent {
            market: market.key(),
            outcome,
            final_yield_index,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Redeems winning conditional tokens for original principal PLUS accrued NectarFi yield
    pub fn settle_and_claim_yield(
        ctx: Context<SettleAndClaimYield>,
        burn_amount: u64,
    ) -> Result<()> {
        require!(burn_amount > 0, VaultError::ZeroRedemptionAmount);
        let market = &ctx.accounts.market_account;
        require!(market.is_resolved, VaultError::MarketNotResolved);

        // 1. Burn winning conditional tokens from user
        let (winning_mint_key, user_token_account) = match market.winning_outcome {
            Outcome::Yes => (
                market.yes_token_mint,
                ctx.accounts.user_winning_token_account.to_account_info(),
            ),
            Outcome::No => (
                market.no_token_mint,
                ctx.accounts.user_winning_token_account.to_account_info(),
            ),
            Outcome::Unresolved => return err!(VaultError::MarketNotResolved),
        };

        require_keys_eq!(
            user_token_account.key(),
            ctx.accounts.user_winning_token_account.key(),
            VaultError::InvalidWinningAccount
        );

        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                token::Burn {
                    mint: ctx.accounts.winning_mint.to_account_info(),
                    from: user_token_account,
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            burn_amount,
        )?;

        // 2. Compute Yield Payout
        // Payout = Principal + (Principal * (settled_index - initial_index) / initial_index)
        let index_delta = market
            .settled_yield_per_share
            .checked_sub(market.initial_yield_index)
            .unwrap_or(0);

        let accrued_yield = (burn_amount as u128)
            .checked_mul(index_delta)
            .ok_or(VaultError::MathOverflow)?
            .checked_div(market.initial_yield_index)
            .ok_or(VaultError::MathOverflow)? as u64;

        let gross_payout = burn_amount
            .checked_add(accrued_yield)
            .ok_or(VaultError::MathOverflow)?;

        let fee = gross_payout
            .checked_mul(market.protocol_fee_bps)
            .ok_or(VaultError::MathOverflow)?
            .checked_div(BPS_DIVISOR)
            .ok_or(VaultError::MathOverflow)?;

        let net_payout = gross_payout
            .checked_sub(fee)
            .ok_or(VaultError::MathOverflow)?;

        // 3. Transfer underlying redemption funds from Vault to User
        let seeds = &[
            b"market_config".as_ref(),
            market.market_id.as_bytes(),
            &[market.vault_bump],
        ];
        let signer = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_underlying_reserve.to_account_info(),
                    to: ctx.accounts.user_underlying.to_account_info(),
                    authority: ctx.accounts.vault_escrow.to_account_info(),
                },
                signer,
            ),
            net_payout,
        )?;

        if fee > 0 {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.vault_underlying_reserve.to_account_info(),
                        to: ctx.accounts.protocol_fee_receiver.to_account_info(),
                        authority: ctx.accounts.vault_escrow.to_account_info(),
                    },
                    signer,
                ),
                fee,
            )?;
        }

        emit!(PayoutClaimedEvent {
            market: market.key(),
            user: ctx.accounts.user.key(),
            tokens_burned: burn_amount,
            net_payout,
            accrued_yield,
            fee_charged: fee,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// ACCOUNT VALIDATION STRUCTS
// -----------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(market_id: String)]
pub struct InitializeMarket<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + MarketConfig::MAX_SIZE,
        seeds = [b"market_config", market_id.as_bytes()],
        bump
    )]
    pub market_account: Account<'info, MarketConfig>,

    #[account(
        seeds = [b"vault_escrow", market_id.as_bytes()],
        bump
    )]
    /// CHECK: Program derived account serving as token authority for escrow and mints
    pub vault_escrow: UncheckedAccount<'info>,

    pub underlying_mint: Account<'info, Mint>,
    pub nectar_receipt_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = authority,
        mint::decimals = 6,
        mint::authority = vault_escrow,
    )]
    pub yes_token_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = authority,
        mint::decimals = 6,
        mint::authority = vault_escrow,
    )]
    pub no_token_mint: Account<'info, Mint>,

    /// CHECK: Dominion trusted oracle or crank authority
    pub oracle: UncheckedAccount<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct DepositAndUnderwrite<'info> {
    #[account(
        mut,
        seeds = [b"market_config", market_account.market_id.as_bytes()],
        bump
    )]
    pub market_account: Account<'info, MarketConfig>,

    #[account(
        seeds = [b"vault_escrow", market_account.market_id.as_bytes()],
        bump = market_account.vault_bump
    )]
    /// CHECK: PDA authority
    pub vault_escrow: UncheckedAccount<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        constraint = user_underlying.mint == market_account.underlying_mint,
        constraint = user_underlying.owner == user.key()
    )]
    pub user_underlying: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = nectar_reserve_vault.mint == market_account.underlying_mint
    )]
    pub nectar_reserve_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = nectar_pool_receipt_source.mint == market_account.nectar_receipt_mint
    )]
    pub nectar_pool_receipt_source: Account<'info, TokenAccount>,

    pub nectar_pool_authority: Signer<'info>,

    #[account(
        mut,
        constraint = vault_escrow.key() == vault_escrow.key(),
        constraint = vault_receipt_account.owner == vault_escrow.key(),
        constraint = vault_receipt_account.mint == market_account.nectar_receipt_mint
    )]
    pub vault_receipt_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = yes_token_mint.key() == market_account.yes_token_mint
    )]
    pub yes_token_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = no_token_mint.key() == market_account.no_token_mint
    )]
    pub no_token_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_yes_account.mint == market_account.yes_token_mint,
        constraint = user_yes_account.owner == user.key()
    )]
    pub user_yes_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_no_account.mint == market_account.no_token_mint,
        constraint = user_no_account.owner == user.key()
    )]
    pub user_no_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ResolveMarket<'info> {
    #[account(
        mut,
        seeds = [b"market_config", market_account.market_id.as_bytes()],
        bump,
        has_one = oracle @ VaultError::UnauthorizedOracle
    )]
    pub market_account: Account<'info, MarketConfig>,

    pub oracle: Signer<'info>,
}

#[derive(Accounts)]
pub struct SettleAndClaimYield<'info> {
    #[account(
        seeds = [b"market_config", market_account.market_id.as_bytes()],
        bump
    )]
    pub market_account: Account<'info, MarketConfig>,

    #[account(
        seeds = [b"vault_escrow", market_account.market_id.as_bytes()],
        bump = market_account.vault_bump
    )]
    /// CHECK: PDA signer
    pub vault_escrow: UncheckedAccount<'info>,

    #[account(mut)]
    pub user: Signer<'info>,

    #[account(mut)]
    pub winning_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_winning_token_account.owner == user.key(),
        constraint = user_winning_token_account.mint == winning_mint.key()
    )]
    pub user_winning_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = vault_underlying_reserve.mint == market_account.underlying_mint,
        constraint = vault_underlying_reserve.owner == vault_escrow.key()
    )]
    pub vault_underlying_reserve: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_underlying.mint == market_account.underlying_mint,
        constraint = user_underlying.owner == user.key()
    )]
    pub user_underlying: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = protocol_fee_receiver.mint == market_account.underlying_mint
    )]
    pub protocol_fee_receiver: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

// -----------------------------------------------------------------------------
// STATE STRUCTS & ENUMS
// -----------------------------------------------------------------------------

#[account]
pub struct MarketConfig {
    pub authority: Pubkey,
    pub oracle: Pubkey,
    pub underlying_mint: Pubkey,
    pub nectar_receipt_mint: Pubkey,
    pub yes_token_mint: Pubkey,
    pub no_token_mint: Pubkey,
    pub market_id: String,
    pub vault_bump: u8,
    pub settlement_timestamp: i64,
    pub protocol_fee_bps: u64,
    pub is_resolved: bool,
    pub winning_outcome: Outcome,
    pub total_underwritten_amount: u64,
    pub total_nectar_shares_locked: u64,
    pub initial_yield_index: u128,
    pub settled_yield_per_share: u128,
}

impl MarketConfig {
    pub const MAX_SIZE: usize = 32 + 32 + 32 + 32 + 32 + 32 + (4 + 32) + 1 + 8 + 8 + 1 + 1 + 8 + 8 + 16 + 16 + 64;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Unresolved,
    Yes,
    No,
}

// -----------------------------------------------------------------------------
// EVENTS
// -----------------------------------------------------------------------------

#[event]
pub struct MarketInitializedEvent {
    pub market: Pubkey,
    pub market_id: String,
    pub authority: Pubkey,
    pub settlement_timestamp: i64,
}

#[event]
pub struct UnderwriteDepositEvent {
    pub market: Pubkey,
    pub user: Pubkey,
    pub amount: u64,
    pub shares: u64,
}

#[event]
pub struct MarketResolvedEvent {
    pub market: Pubkey,
    pub outcome: Outcome,
    pub final_yield_index: u128,
    pub timestamp: i64,
}

#[event]
pub struct PayoutClaimedEvent {
    pub market: Pubkey,
    pub user: Pubkey,
    pub tokens_burned: u64,
    pub net_payout: u64,
    pub accrued_yield: u64,
    pub fee_charged: u64,
}

// -----------------------------------------------------------------------------
// ERRORS
// -----------------------------------------------------------------------------

#[error_code]
pub enum VaultError {
    #[msg("Settlement time must be in the future.")]
    InvalidSettlementTime,
    #[msg("Protocol fee exceeds maximum allowed basis points.")]
    ExcessiveFeeBps,
    #[msg("Market identifier is too long. Max 32 chars.")]
    MarketIdTooLong,
    #[msg("Deposit amount must be strictly greater than zero.")]
    ZeroDepositAmount,
    #[msg("Market is already resolved.")]
    MarketAlreadyResolved,
    #[msg("Market trading window has lapsed.")]
    TradingWindowClosed,
    #[msg("Arithmetic overflow in calculation.")]
    MathOverflow,
    #[msg("Invalid market outcome. Must be YES or NO.")]
    InvalidOutcome,
    #[msg("Final yield index cannot be lower than initial yield index.")]
    YieldIndexRegressed,
    #[msg("Signer is not authorized oracle.")]
    UnauthorizedOracle,
    #[msg("Redemption amount must be greater than zero.")]
    ZeroRedemptionAmount,
    #[msg("Market has not yet been resolved.")]
    MarketNotResolved,
    #[msg("Supplied winning token account does not match resolved outcome.")]
    InvalidWinningAccount,
}