# Terminal 1: Start localnet validator with Pyth mock account
solana-test-validator --reset

# Terminal 2: Set Solana config to localnet
solana config set --url localhost

# Build & Deploy
anchor build
anchor deploy --provider.cluster localnet

# Verify deployed program ID
solana program show NectDom111111111111111111111111111111111111