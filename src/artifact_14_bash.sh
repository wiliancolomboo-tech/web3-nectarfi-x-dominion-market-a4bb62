# 1. Clone repository & enter workspace
git clone https://github.com/nectar-fi/nectar-dominion.git
cd nectar-dominion

# 2. Install dependencies
yarn install

# 3. Build Anchor program and generate IDL/types
anchor build

# 4. Run local test validator and comprehensive test suite
anchor test