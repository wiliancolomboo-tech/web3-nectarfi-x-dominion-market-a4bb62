use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Calculation triggered an arithmetic overflow")]
    MathOverflow,
    #[msg("Zero denominator encountered during division")]
    DivisionByZero,
    #[msg("Invalid Pyth oracle account or stale price feed")]
    InvalidOracleFeed,
    #[msg("Pyth price confidence interval is too wide for safe rebalancing")]
    OracleConfidenceTooWide,
    #[msg("Tranche target allocation breached max risk bounds")]
    RiskToleranceExceeded,
    #[msg("Insufficient vault liquidity available for redemption")]
    InsufficientVaultLiquidity,
    #[msg("Junior capital first-loss threshold breached; withdrawals temporarily locked")]
    JuniorTrancheUnderwater,
    #[msg("Invalid authority signature")]
    Unauthorized,
}