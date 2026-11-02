import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { NectarDominion } from "../target/types/nectar_dominion";
import {
  createMint,
  createAccount,
  mintTo,
  getAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { assert, expect } from "chai";

describe("NectarFi x Dominion Market Integration Test Matrix", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.NectarDominion as Program<NectarDominion>;

  const admin = anchor.web3.Keypair.generate();
  const trader = anchor.web3.Keypair.generate();
  const liquidator = anchor.web3.Keypair.generate();

  let nectarShareMint: anchor.web3.Pubkey;
  let borrowMint: anchor.web3.Pubkey;

  let marketPda: anchor.web3.Pubkey;
  let collateralVaultPda: anchor.web3.Pubkey;
  let borrowReservePda: anchor.web3.Pubkey;
  let traderMarginAccountPda: anchor.web3.Pubkey;

  let traderNectarATA: anchor.web3.Pubkey;
  let traderBorrowATA: anchor.web3.Pubkey;
  let liquidatorNectarATA: anchor.web3.Pubkey;
  let liquidatorBorrowATA: anchor.web3.Pubkey;

  const LTV_BPS = new anchor.BN(8000); // 80% LTV
  const LIQ_THRESH_BPS = new anchor.BN(8500); // 85% Liquidation Threshold
  const LIQ_BONUS_BPS = new anchor.BN(500); // 5% Liquidation Bonus

  before(async () => {
    // Airdrop SOL to test actors
    for (const actor of [admin, trader, liquidator]) {
      const sig = await provider.connection.requestAirdrop(
        actor.publicKey,
        10 * anchor.web3.LAMPORTS_PER_SOL
      );
      await provider.connection.confirmTransaction(sig, "confirmed");
    }

    // Deploy test tokens
    nectarShareMint = await createMint(
      provider.connection,
      admin,
      admin.publicKey,
      null,
      6
    );

    borrowMint = await createMint(
      provider.connection,
      admin,
      admin.publicKey,
      null,
      6
    );

    // Derive PDAs
    [marketPda] = anchor.web3.Pubkey.findProgramAddressSync(
      [
        Buffer.from("market"),
        nectarShareMint.toBuffer(),
        borrowMint.toBuffer(),
      ],
      program.programId
    );

    [collateralVaultPda] = anchor.web3.Pubkey.findProgramAddressSync(
      [Buffer.from("collateral_vault"), marketPda.toBuffer()],
      program.programId
    );

    [borrowReservePda] = anchor.web3.Pubkey.findProgramAddressSync(
      [Buffer.from("borrow_reserve"), marketPda.toBuffer()],
      program.programId
    );

    [traderMarginAccountPda] = anchor.web3.Pubkey.findProgramAddressSync(
      [
        Buffer.from("margin_account"),
        marketPda.toBuffer(),
        trader.publicKey.toBuffer(),
      ],
      program.programId
    );

    // Create Token Accounts
    traderNectarATA = await createAccount(
      provider.connection,
      trader,
      nectarShareMint,
      trader.publicKey
    );
    traderBorrowATA = await createAccount(
      provider.connection,
      trader,
      borrowMint,
      trader.publicKey
    );
    liquidatorNectarATA = await createAccount(
      provider.connection,
      liquidator,
      nectarShareMint,
      liquidator.publicKey
    );
    liquidatorBorrowATA = await createAccount(
      provider.connection,
      liquidator,
      borrowMint,
      liquidator.publicKey
    );

    // Mint Nectar tokens to Trader
    await mintTo(
      provider.connection,
      admin,
      nectarShareMint,
      traderNectarATA,
      admin,
      1_000_000_000 // 1,000 nectarTokens
    );

    // Mint USDC liquidity to Liquidator
    await mintTo(
      provider.connection,
      admin,
      borrowMint,
      liquidatorBorrowATA,
      admin,
      50_000_000_000 // 50,000 borrow assets
    );
  });

  it("Initializes the Dominion Market with NectarFi Collateral rules", async () => {
    await program.methods
      .initializeMarket(LTV_BPS, LIQ_THRESH_BPS, LIQ_BONUS_BPS)
      .accounts({
        market: marketPda,
        nectarShareMint,
        borrowMint,
        nectarVaultProgram: anchor.web3.Keypair.generate().publicKey,
        collateralVault: collateralVaultPda,
        borrowReserve: borrowReservePda,
        admin: admin.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([admin])
      .rpc();

    // Fund the borrow reserve with initial liquidity
    await mintTo(
      provider.connection,
      admin,
      borrowMint,
      borrowReservePda,
      admin,
      100_000_000_000 // 100,000 units
    );

    const marketState = await program.account.marketState.fetch(marketPda);
    assert.equal(marketState.loanToValueBps.toNumber(), 8000);
    assert.equal(marketState.liquidationThresholdBps.toNumber(), 8500);
  });

  it("Deposits NectarFi vault shares into trader margin account", async () => {
    const depositAmount = new anchor.BN(500_000_000); // 500 shares

    await program.methods
      .depositCollateral(depositAmount)
      .accounts({
        market: marketPda,
        marginAccount: traderMarginAccountPda,
        collateralVault: collateralVaultPda,
        userNectarAccount: traderNectarATA,
        user: trader.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([trader])
      .rpc();

    const marginAccount = await program.account.marginAccount.fetch(
      traderMarginAccountPda
    );
    assert.equal(
      marginAccount.collateralShares.toString(),
      depositAmount.toString()
    );

    const vaultTokenAccount = await getAccount(
      provider.connection,
      collateralVaultPda
    );
    assert.equal(vaultTokenAccount.amount.toString(), depositAmount.toString());
  });

  it("Borrows credit within safe LTV limits", async () => {
    // Let 1 nectarShare = $1.20 (120,000,000 in 8 decimals)
    // 500 shares = $600 collateral. At 80% LTV = max $480 borrow.
    // Borrow $300 (300,000,000 units) at $1.00 base asset price
    const sharePriceE8 = new anchor.BN(120_000_000);
    const borrowAssetPriceE8 = new anchor.BN(100_000_000);
    const borrowAmount = new anchor.BN(300_000_000);

    await program.methods
      .borrowCredit(borrowAmount, sharePriceE8, borrowAssetPriceE8)
      .accounts({
        market: marketPda,
        marginAccount: traderMarginAccountPda,
        borrowReserve: borrowReservePda,
        userBorrowAccount: traderBorrowATA,
        user: trader.publicKey,
        owner: trader.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([trader])
      .rpc();

    const traderBorrowTokenAcc = await getAccount(
      provider.connection,
      traderBorrowATA
    );
    assert.equal(
      traderBorrowTokenAcc.amount.toString(),
      borrowAmount.toString()
    );

    const marginAccount = await program.account.marginAccount.fetch(
      traderMarginAccountPda
    );
    assert.equal(marginAccount.borrowedDebt.toString(), borrowAmount.toString());
  });

  it("Rejects borrow exceeding allowable LTV capacity", async () => {
    const sharePriceE8 = new anchor.BN(120_000_000);
    const borrowAssetPriceE8 = new anchor.BN(100_000_000);
    const excessiveBorrow = new anchor.BN(250_000_000); // Would push total to $550 > $480 cap

    try {
      await program.methods
        .borrowCredit(excessiveBorrow, sharePriceE8, borrowAssetPriceE8)
        .accounts({
          market: marketPda,
          marginAccount: traderMarginAccountPda,
          borrowReserve: borrowReservePda,
          userBorrowAccount: traderBorrowATA,
          user: trader.publicKey,
          owner: trader.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([trader])
        .rpc();
      assert.fail("Should have thrown borrow capacity exceeded error");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("ExceedsBorrowCapacity");
    }
  });

  it("Executes liquidation when price crash violates liquidation threshold", async () => {
    // Current debt: $300.
    // Price drops: 1 share falls from $1.20 to $0.65.
    // 500 shares * $0.65 = $325.
    // 85% of $325 = $276.25 Liquidation Threshold.
    // Debt ($300) > $276.25 -> Unhealthy position!
    const droppedSharePriceE8 = new anchor.BN(65_000_000);
    const borrowAssetPriceE8 = new anchor.BN(100_000_000);
    const debtToLiquidate = new anchor.BN(150_000_000); // Liquidate $150 debt

    await program.methods
      .liquidatePosition(
        debtToLiquidate,
        droppedSharePriceE8,
        borrowAssetPriceE8
      )
      .accounts({
        market: marketPda,
        marginAccount: traderMarginAccountPda,
        borrowReserve: borrowReservePda,
        collateralVault: collateralVaultPda,
        liquidatorBorrowAccount: liquidatorBorrowATA,
        liquidatorNectarAccount: liquidatorNectarATA,
        liquidator: liquidator.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([liquidator])
      .rpc();

    const liquidatorNectarBalance = await getAccount(
      provider.connection,
      liquidatorNectarATA
    );
    // Liquidator covered $150 debt with 5% bonus ($157.50 collateral)
    // At $0.65/share, seized = 157.50 / 0.65 =~ 242.307692 shares (242,307,692)
    assert.isAbove(Number(liquidatorNectarBalance.amount), 0);

    const marginAccount = await program.account.marginAccount.fetch(
      traderMarginAccountPda
    );
    assert.equal(
      marginAccount.borrowedDebt.toString(),
      new anchor.BN(150_000_000).toString()
    );
  });
});