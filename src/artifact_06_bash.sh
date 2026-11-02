# 1. Clone repository and install dependencies
git clone https://github.com/nectarfi-dominion/dnye-protocol.git
cd dnye-protocol
yarn install

# 2. Build Anchor Smart Contracts
anchor build

# 3. Deploy to Solana Devnet / Mainnet-Beta
solana config set --url https://api.devnet.solana.com
solana airdrop 2
anchor deploy --program-name dominion_nectar_engine

# 4. Verify deployment
anchor idl init --filepath target/idl/dominion_nectar_engine.json DNYE111111111111111111111111111111111111111