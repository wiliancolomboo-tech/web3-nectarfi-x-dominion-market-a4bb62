use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("NectDom111111111111111111111111111111111111");

pub const BPS_DENOMINATOR: u64 = 10_000;
pub const PRICE_DECIMALS: u32 = 8;
pub const SCALING_FACTOR_E8: u128 = 100_000_000;

#[program]
pub mod nectar_dominion {
    use super::*;

    /// Initializes a new Dominion Market paired with a NectarFi yield vault.
    pub fn initialize_market(
        ctx: Context<InitializeMarket>,
        loan_to_value_bps: u64,
        liquidation_threshold_bps: u64,
        liquidation_bonus_bps: u64,
    ) -> Result<()> {
        require!(
            loan_to_value_bps < liquidation_threshold_bps,
            ErrorCode::InvalidRiskParameters
        );
        require!(
            liquidation_threshold_bps <= BPS_DENOMINATOR,
            ErrorCode::InvalidRiskParameters
        );

        let market = &mut ctx.accounts.market;
        market.admin = ctx.accounts.admin.key();
        market.nectar_share_mint = ctx.accounts.nectar_share_mint.key();
        market.borrow_mint = ctx.accounts.borrow_mint.key();
        market.nectar_vault_program = ctx.accounts.nectar_vault_program.key();
        market.collateral_vault = ctx.accounts.collateral_vault.key();
        market.borrow_reserve = ctx.accounts.borrow_reserve.key();
        market.loan_to_value_bps = loan_to_value_bps;
        market.liquidation_threshold_bps = liquidation_threshold_bps;
        market.liquidation_bonus_bps = liquidation_bonus_bps;
        market.total_borrowed = 0;
        market.total_collateral_shares = 0;
        market.bump = ctx.bumps.market;

        emit!(MarketInitializedEvent {
            market: market.key(),
            admin: market.admin,
            nectar_share_mint: market.nectar_share_mint,
            borrow_mint: market.borrow_mint,
        });

        Ok(())
    }

    /// Deposits NectarFi yield-bearing tokens (e.g. nectarUSDC, nectarSOL) as margin collateral.
    pub fn deposit_collateral(ctx: Context<DepositCollateral>, shares_amount: u64) -> Result<()> {
        require!(shares_amount > 0, ErrorCode::ZeroAmount);

        // CPI transfer from user ATA to Dominion Vault ATA
        let cpi_accounts = Transfer {
            from: ctx.accounts.user_nectar_account.to_account_info(),
            to: ctx.accounts.collateral_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        token::transfer(CpiContext::new(cpi_program, cpi_accounts), shares_amount)?;

        let margin_account = &mut ctx.accounts.margin_account;
        margin_account.owner = ctx.accounts.user.key();
        margin_account.market = ctx.accounts.market.key();
        margin_account.collateral_shares = margin_account
            .collateral_shares
            .checked_add(shares_amount)
            .ok_or(ErrorCode::MathOverflow)?;

        let market = &mut ctx.accounts.market;
        market.total_collateral_shares = market
            .total_collateral_shares
            .checked_add(shares_amount)
            .ok_or(ErrorCode::MathOverflow)?;

        emit!(CollateralDepositedEvent {
            user: ctx.accounts.user.key(),
            shares_amount,
            total_user_shares: margin_account.collateral_shares,
        });

        Ok(())
    }

    /// Borrows base assets (e.g., raw USDC) against NectarFi yield collateral.
    pub fn borrow_credit(
        ctx: Context<BorrowCredit>,
        borrow_amount: u64,
        nectar_share_price_e8: u64,
        borrow_asset_price_e8: u64,
    ) -> Result<()> {
        require!(borrow_amount > 0, ErrorCode::ZeroAmount);
        let market = &ctx.accounts.market;
        let margin_account = &mut ctx.accounts.margin_account;

        // Calculate Collateral Value in USD (8 decimals)
        let effective_collateral_usd = calculate_effective_value(
            margin_account.collateral_shares,
            nectar_share_price_e8,
        )?;

        // Max Borrow Capacity = (Effective Collateral USD * LTV) / 10,000
        let max_borrow_usd = (effective_collateral_usd as u128)
            .checked_mul(market.loan_to_value_bps as u128)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(BPS_DENOMINATOR as u128)
            .ok_or(ErrorCode::MathOverflow)?;

        // Calculate Current Debt USD + New Debt USD
        let new_total_debt = margin_account
            .borrowed_debt
            .checked_add(borrow_amount)
            .ok_or(ErrorCode::MathOverflow)?;

        let total_debt_usd = calculate_asset_value(new_total_debt, borrow_asset_price_e8)?;

        require!(
            total_debt_usd <= max_borrow_usd,
            ErrorCode::ExceedsBorrowCapacity
        );

        margin_account.borrowed_debt = new_total_debt;

        let market_mut = &mut ctx.accounts.market;
        market_mut.total_borrowed = market_mut
            .total_borrowed
            .checked_add(borrow_amount)
            .ok_or(ErrorCode::MathOverflow)?;

        // Transfer borrowed asset from reserve to user
        let market_key = market_mut.key();
        let seeds = &[
            b"market",
            market_mut.nectar_share_mint.as_ref(),
            market_mut.borrow_mint.as_ref(),
            &[market_mut.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_accounts = Transfer {
            from: ctx.accounts.borrow_reserve.to_account_info(),
            to: ctx.accounts.user_borrow_account.to_account_info(),
            authority: market_mut.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        token::transfer(
            CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds),
            borrow_amount,
        )?;

        emit!(CreditBorrowedEvent {
            user: margin_account.owner,
            borrow_amount,
            new_total_debt,
        });

        Ok(())
    }

    /// Repays outstanding borrowed credit to restore margin health.
    pub fn repay_credit(ctx: Context<RepayCredit>, repay_amount: u64) -> Result<()> {
        require!(repay_amount > 0, ErrorCode::ZeroAmount);
        let margin_account = &mut ctx.accounts.margin_account;
        let actual_repay = std::cmp::min(repay_amount, margin_account.borrowed_debt);

        let cpi_accounts = Transfer {
            from: ctx.accounts.user_borrow_account.to_account_info(),
            to: ctx.accounts.borrow_reserve.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        token::transfer(CpiContext::new(cpi_program, cpi_accounts), actual_repay)?;

        margin_account.borrowed_debt = margin_account
            .borrowed_debt
            .checked_sub(actual_repay)
            .ok_or(ErrorCode::MathOverflow)?;

        let market = &mut ctx.accounts.market;
        market.total_borrowed = market
            .total_borrowed
            .checked_sub(actual_repay)
            .ok_or(ErrorCode::MathOverflow)?;

        emit!(CreditRepaidEvent {
            user: margin_account.owner,
            repaid_amount: actual_repay,
            remaining_debt: margin_account.borrowed_debt,
        });

        Ok(())
    }

    /// Liquidates an unhealthy position with atomic liquidation bonus.
    pub fn liquidate_position(
        ctx: Context<LiquidatePosition>,
        debt_to_cover: u64,
        nectar_share_price_e8: u64,
        borrow_asset_price_e8: u64,
    ) -> Result<()> {
        require!(debt_to_cover > 0, ErrorCode::ZeroAmount);
        let market = &ctx.accounts.market;
        let margin_account = &mut ctx.accounts.margin_account;

        let effective_collateral_usd = calculate_effective_value(
            margin_account.collateral_shares,
            nectar_share_price_e8,
        )?;
        let total_debt_usd =
            calculate_asset_value(margin_account.borrowed_debt, borrow_asset_price_e8)?;

        // Liquidation Threshold evaluation
        let liquidation_threshold_usd = (effective_collateral_usd as u128)
            .checked_mul(market.liquidation_threshold_bps as u128)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(BPS_DENOMINATOR as u128)
            .ok_or(ErrorCode::MathOverflow)?;

        require!(
            total_debt_usd > liquidation_threshold_usd,
            ErrorCode::PositionIsHealthy
        );

        let repay_amount = std::cmp::min(debt_to_cover, margin_account.borrowed_debt);
        let debt_usd_covered = calculate_asset_value(repay_amount, borrow_asset_price_e8)?;

        // Collateral value to seize with liquidation bonus
        let bonus_multiplier = (BPS_DENOMINATOR as u128)
            .checked_add(market.liquidation_bonus_bps as u128)
            .ok_or(ErrorCode::MathOverflow)?;
        let collateral_usd_to_seize = debt_usd_covered
            .checked_mul(bonus_multiplier)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(BPS_DENOMINATOR as u128)
            .ok_or(ErrorCode::MathOverflow)?;

        // Convert USD seized into Nectar shares: (USD * 10^8) / share_price_e8
        let shares_to_seize = collateral_usd_to_seize
            .checked_mul(SCALING_FACTOR_E8)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(nectar_share_price_e8 as u128)
            .ok_or(ErrorCode::MathOverflow)? as u64;

        require!(
            shares_to_seize <= margin_account.collateral_shares,
            ErrorCode::InsufficientCollateralToSeize
        );

        // 1. Liquidator transfers debt repayment into reserve
        let cpi_repay = Transfer {
            from: ctx.accounts.liquidator_borrow_account.to_account_info(),
            to: ctx.accounts.borrow_reserve.to_account_info(),
            authority: ctx.accounts.liquidator.to_account_info(),
        };
        token::transfer(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_repay),
            repay_amount,
        )?;

        // 2. Market transfers seized Nectar shares to liquidator
        let seeds = &[
            b"market",
            market.nectar_share_mint.as_ref(),
            market.borrow_mint.as_ref(),
            &[market.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_seize = Transfer {
            from: ctx.accounts.collateral_vault.to_account_info(),
            to: ctx.accounts.liquidator_nectar_account.to_account_info(),
            authority: ctx.accounts.market.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_seize,
                signer_seeds,
            ),
            shares_to_seize,
        )?;

        // State mutations
        margin_account.borrowed_debt = margin_account
            .borrowed_debt
            .checked_sub(repay_amount)
            .ok_or(ErrorCode::MathOverflow)?;
        margin_account.collateral_shares = margin_account
            .collateral_shares
            .checked_sub(shares_to_seize)
            .ok_or(ErrorCode::MathOverflow)?;

        let market_mut = &mut ctx.accounts.market;
        market_mut.total_borrowed = market_mut
            .total_borrowed
            .checked_sub(repay_amount)
            .ok_or(ErrorCode::MathOverflow)?;
        market_mut.total_collateral_shares = market_mut
            .total_collateral_shares
            .checked_sub(shares_to_seize)
            .ok_or(ErrorCode::MathOverflow)?;

        emit!(PositionLiquidatedEvent {
            borrower: margin_account.owner,
            liquidator: ctx.accounts.liquidator.key(),
            repaid_amount: repay_amount,
            seized_shares: shares_to_seize,
        });

        Ok(())
    }
}

// ---------------------------------------------------------
// PURE UTILITY MATH FUNCTIONS
// ---------------------------------------------------------

#[inline(always)]
pub fn calculate_effective_value(shares: u64, share_price_e8: u64) -> Result<u128> {
    (shares as u128)
        .checked_mul(share_price_e8 as u128)
        .ok_or_else(|| error!(ErrorCode::MathOverflow))?
        .checked_div(SCALING_FACTOR_E8)
        .ok_or_else(|| error!(ErrorCode::MathOverflow))
}

#[inline(always)]
pub fn calculate_asset_value(amount: u64, price_e8: u64) -> Result<u128> {
    (amount as u128)
        .checked_mul(price_e8 as u128)
        .ok_or_else(|| error!(ErrorCode::MathOverflow))?
        .checked_div(SCALING_FACTOR_E8)
        .ok_or_else(|| error!(ErrorCode::MathOverflow))
}

// ---------------------------------------------------------
// INSTRUCTION CONTEXTS & STRUCTS
// ---------------------------------------------------------

#[derive(Accounts)]
pub fn initialize_market_validation(ctx: &Context<InitializeMarket>) -> Result<()> {
    Ok(())
}

#[derive(Accounts)]
pub struct InitializeMarket<'info> {
    #[account(
        init,
        payer = admin,
        space = 8 + MarketState::INIT_SPACE,
        seeds = [b"market", nectar_share_mint.key().as_ref(), borrow_mint.key().as_ref()],
        bump
    )]
    pub market: Account<'info, MarketState>,

    pub nectar_share_mint: Account<'info, Mint>,
    pub borrow_mint: Account<'info, Mint>,

    /// CHECK: NectarFi Vault program or controller address verified via governance config
    pub nectar_vault_program: UncheckedAccount<'info>,

    #[account(
        init,
        payer = admin,
        token::mint = nectar_share_mint,
        token::authority = market,
        seeds = [b"collateral_vault", market.key().as_ref()],
        bump
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = admin,
        token::mint = borrow_mint,
        token::authority = market,
        seeds = [b"borrow_reserve", market.key().as_ref()],
        bump
    )]
    pub borrow_reserve: Account<'info, TokenAccount>,

    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct DepositCollateral<'info> {
    #[account(
        mut,
        seeds = [b"market", market.nectar_share_mint.as_ref(), market.borrow_mint.as_ref()],
        bump = market.bump
    )]
    pub market: Account<'info, MarketState>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + MarginAccount::INIT_SPACE,
        seeds = [b"margin_account", market.key().as_ref(), user.key().as_ref()],
        bump
    )]
    pub margin_account: Account<'info, MarginAccount>,

    #[account(
        mut,
        address = market.collateral_vault
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_nectar_account.mint == market.nectar_share_mint @ ErrorCode::InvalidMint,
        constraint = user_nectar_account.owner == user.key() @ ErrorCode::Unauthorized
    )]
    pub user_nectar_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct BorrowCredit<'info> {
    #[account(
        mut,
        seeds = [b"market", market.nectar_share_mint.as_ref(), market.borrow_mint.as_ref()],
        bump = market.bump
    )]
    pub market: Account<'info, MarketState>,

    #[account(
        mut,
        seeds = [b"margin_account", market.key().as_ref(), user.key().as_ref()],
        bump,
        has_one = owner @ ErrorCode::Unauthorized
    )]
    pub margin_account: Account<'info, MarginAccount>,

    #[account(
        mut,
        address = market.borrow_reserve
    )]
    pub borrow_reserve: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_borrow_account.mint == market.borrow_mint @ ErrorCode::InvalidMint,
        constraint = user_borrow_account.owner == user.key() @ ErrorCode::Unauthorized
    )]
    pub user_borrow_account: Account<'info, TokenAccount>,

    pub user: Signer<'info>,
    #[account(address = margin_account.owner)]
    pub owner: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RepayCredit<'info> {
    #[account(
        mut,
        seeds = [b"market", market.nectar_share_mint.as_ref(), market.borrow_mint.as_ref()],
        bump = market.bump
    )]
    pub market: Account<'info, MarketState>,

    #[account(
        mut,
        seeds = [b"margin_account", market.key().as_ref(), user.key().as_ref()],
        bump
    )]
    pub margin_account: Account<'info, MarginAccount>,

    #[account(
        mut,
        address = market.borrow_reserve
    )]
    pub borrow_reserve: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_borrow_account.mint == market.borrow_mint @ ErrorCode::InvalidMint,
        constraint = user_borrow_account.owner == user.key() @ ErrorCode::Unauthorized
    )]
    pub user_borrow_account: Account<'info, TokenAccount>,

    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct LiquidatePosition<'info> {
    #[account(
        mut,
        seeds = [b"market", market.nectar_share_mint.as_ref(), market.borrow_mint.as_ref()],
        bump = market.bump
    )]
    pub market: Account<'info, MarketState>,

    #[account(
        mut,
        seeds = [b"margin_account", market.key().as_ref(), margin_account.owner.as_ref()],
        bump
    )]
    pub margin_account: Account<'info, MarginAccount>,

    #[account(
        mut,
        address = market.borrow_reserve
    )]
    pub borrow_reserve: Account<'info, TokenAccount>,

    #[account(
        mut,
        address = market.collateral_vault
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = liquidator_borrow_account.mint == market.borrow_mint @ ErrorCode::InvalidMint,
        constraint = liquidator_borrow_account.owner == liquidator.key() @ ErrorCode::Unauthorized
    )]
    pub liquidator_borrow_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = liquidator_nectar_account.mint == market.nectar_share_mint @ ErrorCode::InvalidMint,
        constraint = liquidator_nectar_account.owner == liquidator.key() @ ErrorCode::Unauthorized
    )]
    pub liquidator_nectar_account: Account<'info, TokenAccount>,

    pub liquidator: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

// ---------------------------------------------------------
// STATE SCHEMAS
// ---------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct MarketState {
    pub admin: Pubkey,
    pub nectar_share_mint: Pubkey,
    pub borrow_mint: Pubkey,
    pub nectar_vault_program: Pubkey,
    pub collateral_vault: Pubkey,
    pub borrow_reserve: Pubkey,
    pub loan_to_value_bps: u64,
    pub liquidation_threshold_bps: u64,
    pub liquidation_bonus_bps: u64,
    pub total_borrowed: u64,
    pub total_collateral_shares: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct MarginAccount {
    pub owner: Pubkey,
    pub market: Pubkey,
    pub collateral_shares: u64,
    pub borrowed_debt: u64,
}

// ---------------------------------------------------------
// EVENTS & ERRORS
// ---------------------------------------------------------

#[event]
pub struct MarketInitializedEvent {
    pub market: Pubkey,
    pub admin: Pubkey,
    pub nectar_share_mint: Pubkey,
    pub borrow_mint: Pubkey,
}

#[event]
pub struct CollateralDepositedEvent {
    pub user: Pubkey,
    pub shares_amount: u64,
    pub total_user_shares: u64,
}

#[event]
pub struct CreditBorrowedEvent {
    pub user: Pubkey,
    pub borrow_amount: u64,
    pub new_total_debt: u64,
}

#[event]
pub struct CreditRepaidEvent {
    pub user: Pubkey,
    pub repaid_amount: u64,
    pub remaining_debt: u64,
}

#[event]
pub struct PositionLiquidatedEvent {
    pub borrower: Pubkey,
    pub liquidator: Pubkey,
    pub repaid_amount: u64,
    pub seized_shares: u64,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Risk parameters invalid: LTV must be lower than Liquidation Threshold")]
    InvalidRiskParameters,
    #[msg("Instruction amount must be greater than zero")]
    ZeroAmount,
    #[msg("Safe arithmetic overflow or underflow occurred")]
    MathOverflow,
    #[msg("Borrow request exceeds maximum allowable Loan-To-Value")]
    ExceedsBorrowCapacity,
    #[msg("Position health is above liquidation threshold")]
    PositionIsHealthy,
    #[msg("Insufficient collateral available in user margin position to seize")]
    InsufficientCollateralToSeize,
    #[msg("Token account mint does not match market configuration")]
    InvalidMint,
    #[msg("Unauthorized signer for the target margin account")]
    Unauthorized,
}