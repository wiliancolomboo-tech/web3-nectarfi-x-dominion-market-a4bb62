# 1. Clone the repository and enter directory
git clone https://github.com/nectar-dominion/nectar-dominion-core.git
cd nectar-dominion-core

# 2. Install NPM dependencies
yarn install

# 3. Synchronize program keys
anchor keys sync

# 4. Compile the Anchor Program
anchor build

# 5. Run Localnet Integration Tests
anchor test