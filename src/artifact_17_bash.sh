# Start a clean Solana test validator
solana-test-validator --reset

# In another terminal window: Deploy Program
anchor deploy --provider.cluster localnet

# Initialize market via CLI / TypeScript script
npx ts-node scripts/init_market.ts \
  --market-id "FED_RATE_DECISION_Q3_2025" \
  --collateral-mint "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" \
  --oracle "DominionOrac1e1111111111111111111111111111"