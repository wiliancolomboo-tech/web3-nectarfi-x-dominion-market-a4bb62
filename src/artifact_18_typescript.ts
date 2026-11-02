import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { NectarDominion } from "../target/types/nectar_dominion";
import {
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { Keypair, LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";
import { expect } from "chai";

describe("nectar-dominion tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.NectarDominion as Program<NectarDominion>;

  const admin = Keypair.generate();
  const seniorLp = Keypair.generate();
  const juniorLp = Keypair.generate();
  const borrower = Keypair.generate();
  const liquidator = Keypair.generate();

  let underlyingMint: PublicKey;
  let collateralMint: PublicKey;

  let marketPda: PublicKey;
  let vaultUnderlyingPda: PublicKey;
  let seniorMintPda: PublicKey;
  let juniorMintPda: PublicKey;

  let seniorLpUnderlyingAta: PublicKey;
  let seniorLpReceiptAta: PublicKey;

  let juniorLpUnderlyingAta: PublicKey;
  let juniorLpReceiptAta: PublicKey;

  let borrowerCollateralAta: PublicKey;
  let borrowerUnderlyingAta: PublicKey;
  let obligationPda: PublicKey;
  let collateralVaultPda: PublicKey;

  const marketId = new anchor.BN(888);
  const seniorFixedApyBps = new anchor.BN(600); // 6.00%
  const ltvBps = new anchor.BN(7000); // 70%
  const liquidationThresholdBps = new anchor.BN(8000); // 80%

  before(async () => {
    // Airdrop SOL to participants
    const accounts = [admin, seniorLp, juniorLp, borrower, liquidator];
    for (const acc of accounts) {
      const sig = await provider.connection.requestAirdrop(
        acc.publicKey,
        10 * LAMPORTS_PER_SOL
      );
      const latestBlockhash = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: sig,
        ...latestBlockhash,
      });
    }

    // Create Underlying Mint (USDC - 6 decimals) & Collateral Mint (SOL - 9 decimals)
    underlyingMint = await createMint(
      provider.connection,
      admin,
      admin.publicKey,
      null,
      6
    );
    collateralMint = await createMint(
      provider.connection,
      admin,
      admin.publicKey,
      null,
      9
    );

    // Derive Market PDA addresses
    [marketPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("market"), marketId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    [vaultUnderlyingPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("escrow"), marketPda.toBuffer()],
      program.programId
    );

    [seniorMintPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("senior_mint"), marketPda.toBuffer()],
      program.programId
    );

    [juniorMintPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("junior_mint"), marketPda.toBuffer()],
      program.programId
    );

    // Setup Token Accounts
    seniorLpUnderlyingAta = await createAccount(
      provider.connection,
      seniorLp,
      underlyingMint,
      seniorLp.publicKey
    );
    seniorLpReceiptAta = await createAccount(
      provider.connection,
      seniorLp,
      seniorMintPda,
      seniorLp.publicKey
    );

    juniorLpUnderlyingAta = await createAccount(
      provider.connection,
      juniorLp,
      underlyingMint,
      juniorLp.publicKey
    );
    juniorLpReceiptAta = await createAccount(
      provider.connection,
      juniorLp,
      juniorMintPda,
      juniorLp.publicKey
    );

    borrowerCollateralAta = await createAccount(
      provider.connection,
      borrower,
      collateralMint,
      borrower.publicKey
    );
    borrowerUnderlyingAta = await createAccount(
      provider.connection,
      borrower,
      underlyingMint,
      borrower.publicKey
    );

    [obligationPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("obligation"),
        borrower.publicKey.toBuffer(),
        marketPda.toBuffer(),
      ],
      program.programId
    );

    [collateralVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("collateral_vault"), obligationPda.toBuffer()],
      program.programId
    );

    // Mint test supplies
    await mintTo(
      provider.connection,
      admin,
      underlyingMint,
      seniorLpUnderlyingAta,
      admin,
      1_000_000 * 1e6
    );
    await mintTo(
      provider.connection,
      admin,
      underlyingMint,
      juniorLpUnderlyingAta,
      admin,
      500_000 * 1e6
    );
    await mintTo(
      provider.connection,
      admin,
      collateralMint,
      borrowerCollateralAta,
      admin,
      100 * 1e9
    );
  });

  it("1. Initializes Dominion Fixed-Rate Market PDA", async () => {
    const futureMaturity = new anchor.BN(
      Math.floor(Date.now() / 1000) + 86400 * 30
    ); // 30 Days

    await program.methods
      .initializeMarket(
        marketId,
        seniorFixedApyBps,
        ltvBps,
        liquidationThresholdBps,
        futureMaturity
      )
      .accounts({
        admin: admin.publicKey,
        underlyingMint,
        collateralMint,
      })
      .signers([admin])
      .rpc();

    const marketState = await program.account.dominionMarket.fetch(marketPda);
    expect(marketState.marketId.toNumber()).to.eq(888);
    expect(marketState.seniorFixedApyBps.toNumber()).to.eq(600);
    expect(marketState.totalSeniorDeposited.toNumber()).to.eq(0);
  });

  it("2. Deposits Liquidity into Senior Tranche (Receives dSNR)", async () => {
    const depositAmount = new anchor.BN(100_000 * 1e6);

    await program.methods
      .depositSenior(depositAmount)
      .accounts({
        depositor: seniorLp.publicKey,
        market: marketPda,
        depositorTokenAccount: seniorLpUnderlyingAta,
        depositorSeniorReceipt: seniorLpReceiptAta,
      })
      .signers([seniorLp])
      .rpc();

    const seniorReceiptAccount = await getAccount(
      provider.connection,
      seniorLpReceiptAta
    );
    expect(Number(seniorReceiptAccount.amount)).to.eq(100_000 * 1e6);

    const marketState = await program.account.dominionMarket.fetch(marketPda);
    expect(marketState.totalSeniorDeposited.toNumber()).to.eq(100_000 * 1e6);
  });

  it("3. Deposits Capital into Junior Tranche Shield (Receives dJNR)", async () => {
    const juniorDepositAmount = new anchor.BN(50_000 * 1e6);

    await program.methods
      .depositJunior(juniorDepositAmount)
      .accounts({
        depositor: juniorLp.publicKey,
        market: marketPda,
        depositorTokenAccount: juniorLpUnderlyingAta,
        depositorJuniorReceipt: juniorLpReceiptAta,
      })
      .signers([juniorLp])
      .rpc();

    const juniorReceipt = await getAccount(
      provider.connection,
      juniorLpReceiptAta
    );
    expect(Number(juniorReceipt.amount)).to.eq(50_000 * 1e6);
  });

  it("4. Issues Debt Obligation against Locked Collateral", async () => {
    // Lock 10 SOL ($1,500 collateral) to borrow 700 USDC (LTV <= 70%)
    const collateralToLock = new anchor.BN(10 * 1e9);
    const borrowAmount = new anchor.BN(700 * 1e6);

    await program.methods
      .issueDebt(collateralToLock, borrowAmount)
      .accounts({
        borrower: borrower.publicKey,
        market: marketPda,
        collateralMint,
        borrowerCollateralWallet: borrowerCollateralAta,
        borrowerUnderlyingWallet: borrowerUnderlyingAta,
      })
      .signers([borrower])
      .rpc();

    const obligation = await program.account.borrowerObligation.fetch(
      obligationPda
    );
    expect(obligation.collateralAmount.toNumber()).to.eq(10 * 1e9);
    expect(obligation.borrowedAmount.toNumber()).to.eq(700 * 1e6);

    const borrowerUnderlying = await getAccount(
      provider.connection,
      borrowerUnderlyingAta
    );
    expect(Number(borrowerUnderlying.amount)).to.eq(700 * 1e6);
  });

  it("5. Settles Senior Waterfall Interest Calculation", async () => {
    await program.methods
      .settleWaterfall()
      .accounts({
        keeper: admin.publicKey,
        market: marketPda,
      })
      .signers([admin])
      .rpc();

    const marketState = await program.account.dominionMarket.fetch(marketPda);
    expect(marketState.lastSettlementTimestamp.toNumber()).to.be.greaterThan(0);
  });

  it("6. Repays Debt and Releases Locked Collateral", async () => {
    const repayAmount = new anchor.BN(700 * 1e6);

    await program.methods
      .repayDebt(repayAmount)
      .accounts({
        borrower: borrower.publicKey,
        market: marketPda,
        obligation: obligationPda,
        collateralVault: collateralVaultPda,
        vaultUnderlying: vaultUnderlyingPda,
        borrowerUnderlyingWallet: borrowerUnderlyingAta,
        borrowerCollateralWallet: borrowerCollateralAta,
      })
      .signers([borrower])
      .rpc();

    const obligation = await program.account.borrowerObligation.fetch(
      obligationPda
    );
    expect(obligation.borrowedAmount.toNumber()).to.eq(0);
    expect(obligation.collateralAmount.toNumber()).to.eq(0);
  });
});