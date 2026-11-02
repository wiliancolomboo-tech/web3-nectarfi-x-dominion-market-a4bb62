// Script snippet or Anchor REPL
const tx = await program.methods
  .initializeMarket(
    new anchor.BN(1), // market_id
    new anchor.BN(8000), // 80% LTV
    new anchor.BN(8500), // 85% Liquidation Threshold
    new anchor.BN(500),  // 5% Liquidator Bonus
    new anchor.BN(600)   // 6% Fixed Interest APR
  )
  .accounts({ ... })
  .rpc();
console.log("Market Initialized. Transaction Signature:", tx);