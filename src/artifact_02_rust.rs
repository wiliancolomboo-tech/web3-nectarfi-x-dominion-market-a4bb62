use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer, MintTo, Burn};

declare_id!("NectDom1111111111111111111111111111111111111");

pub const BPS_DIVISOR: u64 = 10_000;
pub const SECONDS_PER_YEAR: i64 = 31_536_000;

#[program]
pub mod nectar_dominion {
    use super::*;

    /// Initialize a new dual-tranche debt market vault linked to a Dominion Market instrument.
    pub fn initialize_vault(
        ctx: Context<InitializeVault>,
        senior_fixed_apy_bps: u64,
        reserve_ratio_junior_bps: u64,
        maturity_timestamp: i64,
        max_capacity: u64,
    ) -> Result<()> {
        require!(senior_fixed_apy_bps <= 5_000, CustomError::ExcessiveSeniorRate); // Max 50%
        require!(reserve_ratio_junior_bps <= 10_000, CustomError::InvalidRatio);
        require!(
            maturity_timestamp > Clock::get()?.unix_timestamp,
            CustomError::InvalidMaturity
        );

        let vault = &mut ctx.accounts.vault;
        vault.authority = ctx.accounts.authority.key();
        vault.underlying_mint = ctx.accounts.underlying_mint.key();
        vault.senior_mint = ctx.accounts.senior_mint.key();
        vault.junior_mint = ctx.accounts.junior_mint.key();
        vault.vault_underlying_reserve = ctx.accounts.vault_underlying_reserve.key();
        vault.dominion_market = ctx.accounts.dominion_market.key();
        vault.dominion_note_mint = ctx.accounts.dominion_note_mint.key();

        vault.senior_fixed_apy_bps = senior_fixed_apy_bps;
        vault.reserve_ratio_junior_bps = reserve_ratio_junior_bps;
        vault.maturity_timestamp = maturity_timestamp;
        vault.max_capacity = max_capacity;

        vault.senior_principal_deposited = 0;
        vault.junior_principal_deposited = 0;
        vault.senior_yield_settled = 0;
        vault.is_settled = false;
        vault.is_frozen = false;
        vault.bump = ctx.bumps.vault;
        vault.reserve_bump = ctx.bumps.vault_underlying_reserve;

        emit!(VaultInitializedEvent {
            vault: vault.key(),
            maturity_timestamp,
            senior_fixed_apy_bps,
        });

        Ok(())
    }

    /// Deposit into Senior Tranche (Capital protected, fixed coupon priority).
    pub fn deposit_senior(ctx: Context<DepositSenior>, amount: u64) -> Result<()> {
        require!(amount > 0, CustomError::ZeroAmount);
        let vault = &mut ctx.accounts.vault;
        let clock = Clock::get()?;
        require!(!vault.is_settled, CustomError::VaultAlreadySettled);
        require!(!vault.is_frozen, CustomError::VaultFrozen);
        require!(clock.unix_timestamp < vault.maturity_timestamp, CustomError::MaturityPassed);

        let total_deposits = vault.senior_principal_deposited.checked_add(amount)
            .ok_or(CustomError::MathOverflow)?;
        require!(
            total_deposits.checked_add(vault.junior_principal_deposited).unwrap() <= vault.max_capacity,
            CustomError::ExceedsMaxCapacity
        );

        // Enforce junior buffer safety requirement
        let required_junior = (total_deposits as u128)
            .checked_mul(vault.reserve_ratio_junior_bps as u128).unwrap()
            .checked_div(BPS_DIVISOR as u128).unwrap() as u64;
        require!(
            vault.junior_principal_deposited >= required_junior,
            CustomError::InsufficientJuniorProtectionBuffer
        );

        // Transfer underlying token from depositor to vault reserve
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.user_underlying_account.to_account_info(),
                    to: ctx.accounts.vault_underlying_reserve.to_account_info(),
                    authority: ctx.accounts.depositor.to_account_info(),
                },
            ),
            amount,
        )?;

        // Mint senior share tokens 1:1 against initial deposit
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault",
            vault.dominion_market.as_ref(),
            &[vault.bump],
        ]];
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.senior_mint.to_account_info(),
                    to: ctx.accounts.user_senior_account.to_account_info(),
                    authority: vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        vault.senior_principal_deposited = total_deposits;

        emit!(SeniorDepositedEvent {
            depositor: ctx.accounts.depositor.key(),
            amount,
            total_senior: vault.senior_principal_deposited,
        });

        Ok(())
    }

    /// Deposit into Junior Tranche (First loss capital, receives residual excess return).
    pub fn deposit_junior(ctx: Context<DepositJunior>, amount: u64) -> Result<()> {
        require!(amount > 0, CustomError::ZeroAmount);
        let vault = &mut ctx.accounts.vault;
        let clock = Clock::get()?;
        require!(!vault.is_settled, CustomError::VaultAlreadySettled);
        require!(!vault.is_frozen, CustomError::VaultFrozen);
        require!(clock.unix_timestamp < vault.maturity_timestamp, CustomError::MaturityPassed);

        let total_junior = vault.junior_principal_deposited.checked_add(amount)
            .ok_or(CustomError::MathOverflow)?;
        require!(
            vault.senior_principal_deposited.checked_add(total_junior).unwrap() <= vault.max_capacity,
            CustomError::ExceedsMaxCapacity
        );

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.user_underlying_account.to_account_info(),
                    to: ctx.accounts.vault_underlying_reserve.to_account_info(),
                    authority: ctx.accounts.depositor.to_account_info(),
                },
            ),
            amount,
        )?;

        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault",
            vault.dominion_market.as_ref(),
            &[vault.bump],
        ]];
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.junior_mint.to_account_info(),
                    to: ctx.accounts.user_junior_account.to_account_info(),
                    authority: vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        vault.junior_principal_deposited = total_junior;

        emit!(JuniorDepositedEvent {
            depositor: ctx.accounts.depositor.key(),
            amount,
            total_junior: vault.junior_principal_deposited,
        });

        Ok(())
    }

    /// Deploy aggregated underlying tokens to Dominion Market Note Issuer.
    pub fn deploy_to_dominion(ctx: Context<DeployToDominion>, amount: u64) -> Result<()> {
        let vault = &ctx.accounts.vault;
        require!(!vault.is_settled, CustomError::VaultAlreadySettled);
        require!(!vault.is_frozen, CustomError::VaultFrozen);

        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault",
            vault.dominion_market.as_ref(),
            &[vault.bump],
        ]];

        // Transfer underlying assets from Vault Reserve to Dominion Escrow
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_underlying_reserve.to_account_info(),
                    to: ctx.accounts.dominion_escrow.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        // Receive Dominion Debt Notes in Vault's custody
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.dominion_note_mint.to_account_info(),
                    to: ctx.accounts.vault_note_custody.to_account_info(),
                    authority: ctx.accounts.dominion_program_authority.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        emit!(CapitalDeployedEvent {
            vault: vault.key(),
            dominion_market: vault.dominion_market,
            amount_deployed: amount,
        });

        Ok(())
    }

    /// Settle vault at or post-maturity by redeeming Dominion Notes and computing waterfall.
    pub fn settle_maturity(ctx: Context<SettleMaturity>, total_proceeds_returned: u64) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let clock = Clock::get()?;
        require!(clock.unix_timestamp >= vault.maturity_timestamp, CustomError::MaturityNotReached);
        require!(!vault.is_settled, CustomError::VaultAlreadySettled);

        // Burn the dominion debt notes held by the vault
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault",
            vault.dominion_market.as_ref(),
            &[vault.bump],
        ]];
        let notes_to_burn = ctx.accounts.vault_note_custody.amount;
        if notes_to_burn > 0 {
            token::burn(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Burn {
                        mint: ctx.accounts.dominion_note_mint.to_account_info(),
                        from: ctx.accounts.vault_note_custody.to_account_info(),
                        authority: vault.to_account_info(),
                    },
                    signer_seeds,
                ),
                notes_to_burn,
            )?;
        }

        // Calculate senior guaranteed return
        let senior_principal = vault.senior_principal_deposited;
        let senior_coupon = (senior_principal as u128)
            .checked_mul(vault.senior_fixed_apy_bps as u128).unwrap()
            .checked_div(BPS_DIVISOR as u128).unwrap() as u64;

        let total_senior_obligation = senior_principal.checked_add(senior_coupon).unwrap();

        // Calculate waterfall payouts
        if total_proceeds_returned >= total_senior_obligation {
            vault.senior_yield_settled = senior_coupon;
        } else {
            // Undercollateralized / shortfall scenario: Senior absorbs remaining proceeds, junior gets 0
            vault.senior_yield_settled = if total_proceeds_returned > senior_principal {
                total_proceeds_returned - senior_principal
            } else {
                0
            };
        }

        vault.is_settled = true;

        emit!(VaultSettledEvent {
            vault: vault.key(),
            total_proceeds: total_proceeds_returned,
            senior_obligation: total_senior_obligation,
            settled_timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// Redeem Senior Shares post-settlement.
    pub fn redeem_senior(ctx: Context<RedeemSenior>, share_amount: u64) -> Result<()> {
        let vault = &ctx.accounts.vault;
        require!(vault.is_settled, CustomError::VaultNotSettled);
        require!(share_amount > 0, CustomError::ZeroAmount);

        let senior_total_supply = ctx.accounts.senior_mint.supply;
        require!(senior_total_supply > 0, CustomError::ZeroSupply);

        let total_senior_entitlement = vault.senior_principal_deposited
            .checked_add(vault.senior_yield_settled)
            .ok_or(CustomError::MathOverflow)?;

        let payout = ((share_amount as u128)
            .checked_mul(total_senior_entitlement as u128).unwrap()
            .checked_div(senior_total_supply as u128).unwrap()) as u64;

        // Burn senior shares
        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.senior_mint.to_account_info(),
                    from: ctx.accounts.user_senior_account.to_account_info(),
                    authority: ctx.accounts.redeemer.to_account_info(),
                },
            ),
            share_amount,
        )?;

        // Transfer payout from vault reserve
        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault",
            vault.dominion_market.as_ref(),
            &[vault.bump],
        ]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_underlying_reserve.to_account_info(),
                    to: ctx.accounts.user_underlying_account.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
            payout,
        )?;

        emit!(SeniorRedeemedEvent {
            redeemer: ctx.accounts.redeemer.key(),
            shares_burned: share_amount,
            payout,
        });

        Ok(())
    }

    /// Redeem Junior Shares post-settlement (Yield residual alpha).
    pub fn redeem_junior(ctx: Context<RedeemJunior>, share_amount: u64) -> Result<()> {
        let vault = &ctx.accounts.vault;
        require!(vault.is_settled, CustomError::VaultNotSettled);
        require!(share_amount > 0, CustomError::ZeroAmount);

        let junior_total_supply = ctx.accounts.junior_mint.supply;
        require!(junior_total_supply > 0, CustomError::ZeroSupply);

        let reserve_balance = ctx.accounts.vault_underlying_reserve.amount;
        let senior_total_supply = ctx.accounts.senior_mint.supply;

        let senior_unclaimed_liability = if senior_total_supply > 0 {
            let total_senior = vault.senior_principal_deposited
                .checked_add(vault.senior_yield_settled).unwrap();
            ((senior_total_supply as u128)
                .checked_mul(total_senior as u128).unwrap()
                .checked_div(senior_total_supply as u128).unwrap()) as u64
        } else {
            0
        };

        let junior_pool_remaining = reserve_balance.saturating_sub(senior_unclaimed_liability);

        let payout = ((share_amount as u128)
            .checked_mul(junior_pool_remaining as u128).unwrap()
            .checked_div(junior_total_supply as u128).unwrap()) as u64;

        require!(payout > 0, CustomError::ZeroPayout);

        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.junior_mint.to_account_info(),
                    from: ctx.accounts.user_junior_account.to_account_info(),
                    authority: ctx.accounts.redeemer.to_account_info(),
                },
            ),
            share_amount,
        )?;

        let signer_seeds: &[&[&[u8]]] = &[&[
            b"vault",
            vault.dominion_market.as_ref(),
            &[vault.bump],
        ]];
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_underlying_reserve.to_account_info(),
                    to: ctx.accounts.user_underlying_account.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                signer_seeds,
            ),
            payout,
        )?;

        emit!(JuniorRedeemedEvent {
            redeemer: ctx.accounts.redeemer.key(),
            shares_burned: share_amount,
            payout,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Context Accounts
// -----------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub underlying_mint: Account<'info, Mint>,

    #[account(
        init,
        seeds = [b"vault", dominion_market.key().as_ref()],
        bump,
        payer = authority,
        space = 8 + Vault::LEN
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        init,
        seeds = [b"vault_reserve", vault.key().as_ref()],
        bump,
        payer = authority,
        token::mint = underlying_mint,
        token::authority = vault,
    )]
    pub vault_underlying_reserve: Account<'info, TokenAccount>,

    #[account(
        init,
        seeds = [b"senior_mint", vault.key().as_ref()],
        bump,
        payer = authority,
        mint::decimals = 6,
        mint::authority = vault,
    )]
    pub senior_mint: Account<'info, Mint>,

    #[account(
        init,
        seeds = [b"junior_mint", vault.key().as_ref()],
        bump,
        payer = authority,
        mint::decimals = 6,
        mint::authority = vault,
    )]
    pub junior_mint: Account<'info, Mint>,

    /// CHECK: Dominion market configuration PDA validated by market address
    pub dominion_market: UncheckedAccount<'info>,

    pub dominion_note_mint: Account<'info, Mint>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct DepositSenior<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault.dominion_market.as_ref()],
        bump = vault.bump,
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        mut,
        seeds = [b"vault_reserve", vault.key().as_ref()],
        bump = vault.reserve_bump,
    )]
    pub vault_underlying_reserve: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"senior_mint", vault.key().as_ref()],
        bump,
    )]
    pub senior_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_underlying_account.mint == vault.underlying_mint,
        constraint = user_underlying_account.owner == depositor.key()
    )]
    pub user_underlying_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_senior_account.mint == vault.senior_mint,
        constraint = user_senior_account.owner == depositor.key()
    )]
    pub user_senior_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct DepositJunior<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault.dominion_market.as_ref()],
        bump = vault.bump,
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        mut,
        seeds = [b"vault_reserve", vault.key().as_ref()],
        bump = vault.reserve_bump,
    )]
    pub vault_underlying_reserve: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"junior_mint", vault.key().as_ref()],
        bump,
    )]
    pub junior_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_underlying_account.mint == vault.underlying_mint,
        constraint = user_underlying_account.owner == depositor.key()
    )]
    pub user_underlying_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_junior_account.mint == vault.junior_mint,
        constraint = user_junior_account.owner == depositor.key()
    )]
    pub user_junior_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct DeployToDominion<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        has_one = authority @ CustomError::Unauthorized,
        seeds = [b"vault", vault.dominion_market.as_ref()],
        bump = vault.bump,
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        mut,
        seeds = [b"vault_reserve", vault.key().as_ref()],
        bump = vault.reserve_bump,
    )]
    pub vault_underlying_reserve: Account<'info, TokenAccount>,

    #[account(mut)]
    pub dominion_escrow: Account<'info, TokenAccount>,

    #[account(mut)]
    pub vault_note_custody: Account<'info, TokenAccount>,

    #[account(mut)]
    pub dominion_note_mint: Account<'info, Mint>,

    /// CHECK: PDA of Dominion program verified by authority CPI constraints
    pub dominion_program_authority: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct SettleMaturity<'info> {
    pub crank_keeper: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault.dominion_market.as_ref()],
        bump = vault.bump,
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        mut,
        seeds = [b"vault_reserve", vault.key().as_ref()],
        bump = vault.reserve_bump,
    )]
    pub vault_underlying_reserve: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = vault_note_custody.mint == vault.dominion_note_mint
    )]
    pub vault_note_custody: Account<'info, TokenAccount>,

    #[account(mut)]
    pub dominion_note_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RedeemSenior<'info> {
    #[account(mut)]
    pub redeemer: Signer<'info>,

    #[account(
        seeds = [b"vault", vault.dominion_market.as_ref()],
        bump = vault.bump,
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        mut,
        seeds = [b"vault_reserve", vault.key().as_ref()],
        bump = vault.reserve_bump,
    )]
    pub vault_underlying_reserve: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"senior_mint", vault.key().as_ref()],
        bump,
    )]
    pub senior_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_senior_account.mint == vault.senior_mint,
        constraint = user_senior_account.owner == redeemer.key()
    )]
    pub user_senior_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_underlying_account.mint == vault.underlying_mint,
        constraint = user_underlying_account.owner == redeemer.key()
    )]
    pub user_underlying_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RedeemJunior<'info> {
    #[account(mut)]
    pub redeemer: Signer<'info>,

    #[account(
        seeds = [b"vault", vault.dominion_market.as_ref()],
        bump = vault.bump,
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        mut,
        seeds = [b"vault_reserve", vault.key().as_ref()],
        bump = vault.reserve_bump,
    )]
    pub vault_underlying_reserve: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"junior_mint", vault.key().as_ref()],
        bump,
    )]
    pub junior_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [b"senior_mint", vault.key().as_ref()],
        bump,
    )]
    pub senior_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_junior_account.mint == vault.junior_mint,
        constraint = user_junior_account.owner == redeemer.key()
    )]
    pub user_junior_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_underlying_account.mint == vault.underlying_mint,
        constraint = user_underlying_account.owner == redeemer.key()
    )]
    pub user_underlying_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

// -----------------------------------------------------------------------------
// State Structs
// -----------------------------------------------------------------------------

#[account]
pub struct Vault {
    pub authority: Pubkey,
    pub underlying_mint: Pubkey,
    pub senior_mint: Pubkey,
    pub junior_mint: Pubkey,
    pub vault_underlying_reserve: Pubkey,
    pub dominion_market: Pubkey,
    pub dominion_note_mint: Pubkey,
    pub senior_fixed_apy_bps: u64,
    pub reserve_ratio_junior_bps: u64,
    pub maturity_timestamp: i64,
    pub max_capacity: u64,
    pub senior_principal_deposited: u64,
    pub junior_principal_deposited: u64,
    pub senior_yield_settled: u64,
    pub is_settled: bool,
    pub is_frozen: bool,
    pub bump: u8,
    pub reserve_bump: u8,
}

impl Vault {
    pub const LEN: usize = 32 + 32 + 32 + 32 + 32 + 32 + 32 + 8 + 8 + 8 + 8 + 8 + 8 + 8 + 1 + 1 + 1 + 1 + 64;
}

// -----------------------------------------------------------------------------
// Events
// -----------------------------------------------------------------------------

#[event]
pub struct VaultInitializedEvent {
    pub vault: Pubkey,
    pub maturity_timestamp: i64,
    pub senior_fixed_apy_bps: u64,
}

#[event]
pub struct SeniorDepositedEvent {
    pub depositor: Pubkey,
    pub amount: u64,
    pub total_senior: u64,
}

#[event]
pub struct JuniorDepositedEvent {
    pub depositor: Pubkey,
    pub amount: u64,
    pub total_junior: u64,
}

#[event]
pub struct CapitalDeployedEvent {
    pub vault: Pubkey,
    pub dominion_market: Pubkey,
    pub amount_deployed: u64,
}

#[event]
pub struct VaultSettledEvent {
    pub vault: Pubkey,
    pub total_proceeds: u64,
    pub senior_obligation: u64,
    pub settled_timestamp: i64,
}

#[event]
pub struct SeniorRedeemedEvent {
    pub redeemer: Pubkey,
    pub shares_burned: u64,
    pub payout: u64,
}

#[event]
pub struct JuniorRedeemedEvent {
    pub redeemer: Pubkey,
    pub shares_burned: u64,
    pub payout: u64,
}

// -----------------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------------

#[error_code]
pub enum CustomError {
    #[msg("Senior APY exceeds permitted maximum limit (5000 bps).")]
    ExcessiveSeniorRate,
    #[msg("Invalid reserve ratio provided.")]
    InvalidRatio,
    #[msg("Maturity timestamp must be set in the future.")]
    InvalidMaturity,
    #[msg("Deposit amount must be strictly greater than zero.")]
    ZeroAmount,
    #[msg("The vault has already reached terminal maturity and is settled.")]
    VaultAlreadySettled,
    #[msg("The vault is currently frozen by risk administration.")]
    VaultFrozen,
    #[msg("Maturity timeframe has elapsed. Deposits are closed.")]
    MaturityPassed,
    #[msg("Action exceeds maximum configured capacity for the vault.")]
    ExceedsMaxCapacity,
    #[msg("Deposit breaches Junior Protection Buffer minimum capitalization.")]
    InsufficientJuniorProtectionBuffer,
    #[msg("Signer is not authorized to execute this admin action.")]
    Unauthorized,
    #[msg("Maturity timestamp has not elapsed yet.")]
    MaturityNotReached,
    #[msg("Vault has not been settled yet.")]
    VaultNotSettled,
    #[msg("Zero share token supply detected for redemption.")]
    ZeroSupply,
    #[msg("Redemption calculation yielded 0 payout tokens.")]
    ZeroPayout,
    #[msg("Mathematical calculation overflow.")]
    MathOverflow,
}