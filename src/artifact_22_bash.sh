# 1. Start a persistent test validator in terminal 1
solana-test-validator --reset

# 2. Deploy compiled binary in terminal 2
solana program deploy \
  --program-id target/deploy/nectar_dominion-keypair.json \
  target/deploy/nectar_dominion.so

# 3. Stream program logs
solana logs NecDom1111111111111111111111111111111111111