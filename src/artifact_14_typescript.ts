import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { NectarDominion } from "../target/types/nectar_dominion";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { Keypair, PublicKey } from "@solana/web3.js";
import { assert, expect } from "chai";
import crypto from "crypto";

describe("NectarFi x Dominion Market Protocol Test Suite", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.NectarDominion as Program<NectarDominion>;

  let collateralMint: PublicKey;
  let nectarShareMint: PublicKey;
  let userCollateralAccount: PublicKey;

  const admin = provider.wallet as anchor.Wallet;
  const user = Keypair.generate();
  const oracle = Keypair.generate();

  const marketId = Array.from(crypto.randomBytes(32));

  let marketPda: PublicKey;
  let collateralVaultPda: PublicKey;
  let outcomeAMintPda: PublicKey;
  let outcomeBMintPda: PublicKey;

  before(async () => {
    // 1. Airdrop SOL to test accounts
    const signature = await provider.connection.requestAirdrop(
      user.publicKey,
      10 * anchor.web3.LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(signature);

    // 2. Create mock USDC Collateral Mint
    collateralMint = await createMint(
      provider.connection,
      admin.payer,
      admin.publicKey,
      null,
      6
    );

    // 3. Create mock NectarFi Yield Mint
    nectarShareMint = await createMint(
      provider.connection,
      admin.payer,
      admin.publicKey,
      null,
      6
    );

    // 4. Create user's collateral token account and mint 10,000 USDC
    userCollateralAccount = await createAccount(
      provider.connection,
      admin.payer,
      collateralMint,
      user.publicKey
    );

    await mintTo(
      provider.connection,
      admin.payer,
      collateralMint,
      userCollateralAccount,
      admin.payer,
      10_000_000_000 // 10,000 USDC (6 decimals)
    );

    // 5. Derive PDAs
    [marketPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("dominion_market"), Buffer.from(marketId)],
      program.programId
    );

    [collateralVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("collateral_vault"), marketPda.toBuffer()],
      program.programId
    );

    [outcomeAMintPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("outcome_mint"), marketPda.toBuffer(), Buffer.from([0])],
      program.programId
    );

    [outcomeBMintPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("outcome_mint"), marketPda.toBuffer(), Buffer.from([1])],
      program.programId
    );
  });

  it("1. Initializes a new Nectar-Dominion outcome market", async () => {
    const [configPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("global_config")],
      program.programId
    );
    const [nectarVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("nectar_vault"), marketPda.toBuffer()],
      program.programId
    );

    await program.methods
      .initializeMarket(marketId)
      .accounts({
        creator: admin.publicKey,
        config: configPda,
        market: marketPda,
        collateralMint: collateralMint,
        nectarShareMint: nectarShareMint,
        collateralVault: collateralVaultPda,
        nectarVault: nectarVaultPda,
        outcomeAMint: outcomeAMintPda,
        outcomeBMint: outcomeBMintPda,
        oracle: oracle.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const marketData = await program.account.dominionMarket.fetch(marketPda);
    assert.deepEqual(marketData.marketId, marketId);
    assert.equal(marketData.oracle.toBase58(), oracle.publicKey.toBase58());
  });

  it("2. Splits collateral into YES and NO conditional outcome tokens", async () => {
    const depositAmount = new anchor.BN(1_000_000_000); // 1,000 USDC

    const userOutcomeA = await createAccount(
      provider.connection,
      admin.payer,
      outcomeAMintPda,
      user.publicKey
    );

    const userOutcomeB = await createAccount(
      provider.connection,
      admin.payer,
      outcomeBMintPda,
      user.publicKey
    );

    await program.methods
      .splitCollateral(depositAmount)
      .accounts({
        user: user.publicKey,
        market: marketPda,
        collateralMint: collateralMint,
        userCollateral: userCollateralAccount,
        collateralVault: collateralVaultPda,
        outcomeAMint: outcomeAMintPda,
        outcomeBMint: outcomeBMintPda,
        userOutcomeA: userOutcomeA,
        userOutcomeB: userOutcomeB,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([user])
      .rpc();

    const userOutcomeABalance = await getAccount(provider.connection, userOutcomeA);
    const userOutcomeBBalance = await getAccount(provider.connection, userOutcomeB);

    assert.equal(userOutcomeABalance.amount.toString(), "1000000000");
    assert.equal(userOutcomeBBalance.amount.toString(), "1000000000");
  });

  it("3. Cranks and simulates Nectar yield accrual (Yield Index Increases by 5%)", async () => {
    const newYieldIndex = new anchor.BN("1050000000000"); // 1.05 * 1e12

    await program.methods
      .harvestYield(newYieldIndex)
      .accounts({
        keeper: admin.publicKey,
        market: marketPda,
        nectarProgram: admin.publicKey,
      })
      .rpc();

    const marketData = await program.account.dominionMarket.fetch(marketPda);
    assert.equal(marketData.lastYieldIndex.toString(), "1050000000000");
  });

  it("4. Oracle resolves market with Outcome A (YES) winning", async () => {
    await program.methods
      .resolveMarket(0)
      .accounts({
        oracleOrAdmin: oracle.publicKey,
        market: marketPda,
      })
      .signers([oracle])
      .rpc();

    const marketData = await program.account.dominionMarket.fetch(marketPda);
    assert.equal(marketData.winningOutcome, 0);
  });

  it("5. Settles winning outcome token: Receives Principal + 5% Accrued Yield Bonus", async () => {
    // Fund extra collateral to simulate NectarFi strategy returning yield into collateral vault
    await mintTo(
      provider.connection,
      admin.payer,
      collateralMint,
      collateralVaultPda,
      admin.payer,
      50_000_000 // 50 USDC yield
    );

    const userOutcomeA = await anchor.utils.token.associatedAddress({
      mint: outcomeAMintPda,
      owner: user.publicKey,
    });

    const redeemAmount = new anchor.BN(1_000_000_000); // 1,000 Outcome Tokens

    const balanceBefore = (await getAccount(provider.connection, userCollateralAccount)).amount;

    await program.methods
      .settleRedeem(redeemAmount)
      .accounts({
        user: user.publicKey,
        market: marketPda,
        winningOutcomeMint: outcomeAMintPda,
        userWinningToken: userOutcomeA,
        userCollateral: userCollateralAccount,
        collateralVault: collateralVaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();

    const balanceAfter = (await getAccount(provider.connection, userCollateralAccount)).amount;
    const diff = balanceAfter - balanceBefore;

    // Expected: 1,000 USDC principal + 50 USDC yield (5% of 1,000) = 1,050 USDC
    assert.equal(diff.toString(), "1050000000");
  });
});