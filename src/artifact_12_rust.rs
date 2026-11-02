use anchor_lang::prelude::*;
use crate::state::*;
use crate::errors::DominionNectarError;

#[derive(Accounts)]
pub struct ResolveMarket<'info> {
    pub oracle_or_admin: Signer<'info>,

    #[account(
        mut,
        seeds = [MARKET_SEED, market.market_id.as_ref()],
        bump = market.bump,
    )]
    pub market: Box<Account<'info, DominionMarket>>,
}

pub fn handler(ctx: Context<ResolveMarket>, winning_outcome: u8) -> Result<()> {
    require!(winning_outcome <= 1, DominionNectarError::InvalidOutcomeIndex);
    let market = &mut ctx.accounts.market;

    require!(
        ctx.accounts.oracle_or_admin.key() == market.oracle || ctx.accounts.oracle_or_admin.key() == market.authority,
        DominionNectarError::InvalidOracleSigner
    );
    require!(market.status != MarketStatus::Resolved, DominionNectarError::MarketAlreadyResolved);

    market.status = MarketStatus::Resolved;
    market.winning_outcome = Some(winning_outcome);

    msg!("Market resolved. Winning outcome: {}", winning_outcome);
    Ok(())
}