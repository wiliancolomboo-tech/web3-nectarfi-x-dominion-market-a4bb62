use crate::errors::ErrorCode;
use anchor_lang::prelude::*;

pub const BPS_DENOMINATOR: u128 = 10_000;
pub const SECONDS_PER_YEAR: u128 = 31_536_000;
pub const PRECISION: u128 = 1_000_000_000; // 1e9

pub struct TrancheMath;

impl TrancheMath {
    /// Calculate Senior Accrued Yield for a given delta_time
    pub fn calculate_senior_accrual(
        senior_principal: u64,
        senior_apy_bps: u16,
        elapsed_seconds: i64,
    ) -> Result<u64> {
        if elapsed_seconds <= 0 || senior_principal == 0 {
            return Ok(0);
        }

        let principal_u128 = senior_principal as u128;
        let apy_u128 = senior_apy_bps as u128;
        let time_u128 = elapsed_seconds as u128;

        // Interest = (Principal * APY_bps * Time) / (BPS_DENOMINATOR * SECONDS_PER_YEAR)
        let interest = principal_u128
            .checked_mul(apy_u128)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_mul(time_u128)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(BPS_DENOMINATOR * SECONDS_PER_YEAR)
            .ok_or(ErrorCode::MathOverflow)?;

        u64::try_from(interest).map_err(|_| ErrorCode::MathOverflow.into())
    }

    /// Calculate shares to mint for a deposit
    pub fn calculate_shares_minted(
        deposit_amount: u64,
        total_vault_asset_pool: u64,
        total_existing_shares: u64,
    ) -> Result<u64> {
        if total_existing_shares == 0 || total_vault_asset_pool == 0 {
            return Ok(deposit_amount); // 1:1 initial share ratio
        }

        let shares = (deposit_amount as u128)
            .checked_mul(total_existing_shares as u128)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(total_vault_asset_pool as u128)
            .ok_or(ErrorCode::MathOverflow)?;

        u64::try_from(shares).map_err(|_| ErrorCode::MathOverflow.into())
    }

    /// Calculate assets to return for burned shares
    pub fn calculate_assets_redeemed(
        shares_to_burn: u64,
        total_vault_asset_pool: u64,
        total_existing_shares: u64,
    ) -> Result<u64> {
        if total_existing_shares == 0 {
            return Err(ErrorCode::DivisionByZero.into());
        }

        let assets = (shares_to_burn as u128)
            .checked_mul(total_vault_asset_pool as u128)
            .ok_or(ErrorCode::MathOverflow)?
            .checked_div(total_existing_shares as u128)
            .ok_or(ErrorCode::MathOverflow)?;

        u64::try_from(assets).map_err(|_| ErrorCode::MathOverflow.into())
    }
}