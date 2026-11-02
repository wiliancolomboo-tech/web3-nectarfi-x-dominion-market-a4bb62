# 1. Set configuration to devnet
solana config set --url https://api.devnet.solana.com

# 2. Airdrop deployment gas
solana airdrop 2

# 3. Deploy contract binary
anchor deploy --provider.cluster devnet

# 4. Verify on-chain program
solana program show target/deploy/nectar_dominion-keypair.json