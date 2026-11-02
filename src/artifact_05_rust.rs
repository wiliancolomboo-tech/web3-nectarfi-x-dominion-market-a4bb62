use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum TrancheType {
    Senior,
    Junior,
}

#[account(zero_copy)]
#[repr(C)]
pub struct VaultState {
    pub authority: Pubkey,               // 32
    pub asset_mint: Pubkey,              // 32
    pub dominion_pool: Pubkey,           // 32
    pub reserve_account: Pubkey,         // 32
    pub senior_mint: Pubkey,             // 32
    pub junior_mint: Pubkey,             // 32
    pub pyth_oracle_feed: Pubkey,        // 32
    
    // Financial State (Fixed-point numbers scaled by 1e9)
    pub total_senior_deposited: u64,     // 8
    pub total_junior_deposited: u64,     // 8
    pub total_senior_shares: u64,        // 8
    pub total_junior_shares: u64,        // 8
    pub dominion_active_debt: u64,       // 8
    
    // Risk & Yield Parameters
    pub senior_target_apy_bps: u16,      // 2 (e.g., 800 = 8.00%)
    pub target_utilization_bps: u16,     // 2 (e.g., 8500 = 85.00%)
    pub max_junior_leverage_bps: u16,    // 2 (e.g., 50000 = 5.0x)
    pub last_accrual_timestamp: i64,     // 8
    pub bump: u8,                        // 1
    pub _padding: [u8; 7],               // 7 bytes alignment
}

impl VaultState {
    pub const LEN: usize = 8 + 32*7 + 8*5 + 2*3 + 8 + 1 + 7;
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct InitializeVaultParams {
    pub senior_target_apy_bps: u16,
    pub target_utilization_bps: u16,
    pub max_junior_leverage_bps: u16,
}