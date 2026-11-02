# 1. Build Solana Anchor Program
anchor build

# 2. Deploy to Devnet / Mainnet-Beta
solana program deploy \
  --url https://api.devnet.solana.com \
  --keypair ./deployer-keypair.json \
  ./target/deploy/nectar_dominion_vault.so

# 3. Initialize Environment & Install Node Dependencies
npm install

# 4. Run Full Integration Test Suite with 100% Assertions Passing
anchor test