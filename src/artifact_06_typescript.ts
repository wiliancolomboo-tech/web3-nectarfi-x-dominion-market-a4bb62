import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { NectarDominionVault } from "../target/types/nectar_dominion_vault";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { assert } from "chai";

describe("nectar_dominion_vault integration tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.NectarDominionVault as Program<NectarDominionVault>;
  const wallet = provider.wallet as anchor.Wallet;

  const marketId = "SUPERBOWL_LIX_CHIEFS_WIN";
  let underlyingMint: anchor.web3.PublicKey;
  let nectarReceiptMint: anchor.web3.PublicKey;
  let yesMintKeypair = anchor.web3.Keypair.generate();
  let noMintKeypair = anchor.web3.Keypair.generate();

  let userUnderlyingAccount: anchor.web3.PublicKey;
  let userYesAccount: anchor.web3.PublicKey;
  let userNoAccount: anchor.web3.PublicKey;

  let nectarReserveVault: anchor.web3.PublicKey;
  let nectarPoolReceiptSource: anchor.web3.PublicKey;
  let vaultReceiptAccount: anchor.web3.PublicKey;
  let vaultUnderlyingReserve: anchor.web3.PublicKey;
  let protocolFeeReceiver: anchor.web3.PublicKey;

  let marketConfigPda: anchor.web3.PublicKey;
  let vaultEscrowPda: anchor.web3.PublicKey;
  let vaultBump: number;

  const initialDeposit = new anchor.BN(1_000_000_000); // 1,000 USDC (6 decimals)
  const settlementTimestamp = new anchor.BN(Math.floor(Date.now() / 1000) + 3600); // +1 hour

  before(async () => {
    // 1. Derive PDAs
    [marketConfigPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("market_config"), Buffer.from(marketId)],
      program.programId
    );

    [vaultEscrowPda, vaultBump] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault_escrow"), Buffer.from(marketId)],
      program.programId
    );

    // 2. Create SPL Mints
    underlyingMint = await createMint(
      provider.connection,
      wallet.payer,
      wallet.publicKey,
      null,
      6
    );

    nectarReceiptMint = await createMint(
      provider.connection,
      wallet.payer,
      wallet.publicKey,
      null,
      6
    );

    // 3. Create Token Accounts
    userUnderlyingAccount = await createAccount(
      provider.connection,
      wallet.payer,
      underlyingMint,
      wallet.publicKey
    );

    userYesAccount = await createAccount(
      provider.connection,
      wallet.payer,
      yesMintKeypair.publicKey,
      wallet.publicKey
    );

    userNoAccount = await createAccount(
      provider.connection,
      wallet.payer,
      noMintKeypair.publicKey,
      wallet.publicKey
    );

    nectarReserveVault = await createAccount(
      provider.connection,
      wallet.payer,
      underlyingMint,
      wallet.publicKey
    );

    nectarPoolReceiptSource = await createAccount(
      provider.connection,
      wallet.payer,
      nectarReceiptMint,
      wallet.publicKey
    );

    vaultReceiptAccount = await createAccount(
      provider.connection,
      wallet.payer,
      nectarReceiptMint,
      vaultEscrowPda
    );

    vaultUnderlyingReserve = await createAccount(
      provider.connection,
      wallet.payer,
      underlyingMint,
      vaultEscrowPda
    );

    protocolFeeReceiver = await createAccount(
      provider.connection,
      wallet.payer,
      underlyingMint,
      wallet.publicKey
    );

    // 4. Fund Initial Mint Balances
    await mintTo(
      provider.connection,
      wallet.payer,
      underlyingMint,
      userUnderlyingAccount,
      wallet.payer,
      5_000_000_000
    );

    await mintTo(
      provider.connection,
      wallet.payer,
      nectarReceiptMint,
      nectarPoolReceiptSource,
      wallet.payer,
      5_000_000_000
    );

    // Pre-fund vault underlying reserve for payout liquidity simulation (Principal + Yield)
    await mintTo(
      provider.connection,
      wallet.payer,
      underlyingMint,
      vaultUnderlyingReserve,
      wallet.payer,
      5_000_000_000
    );
  });

  it("1. Successfully initializes Dominion Market with Nectar Vault configuration", async () => {
    const protocolFeeBps = new anchor.BN(50); // 0.50% fee

    await program.methods
      .initializeMarket(marketId, settlementTimestamp, protocolFeeBps)
      .accounts({
        marketAccount: marketConfigPda,
        vaultEscrow: vaultEscrowPda,
        underlyingMint: underlyingMint,
        nectarReceiptMint: nectarReceiptMint,
        yesTokenMint: yesMintKeypair.publicKey,
        noTokenMint: noMintKeypair.publicKey,
        oracle: wallet.publicKey,
        authority: wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([yesMintKeypair, noMintKeypair])
      .rpc();

    const config = await program.account.marketConfig.fetch(marketConfigPda);
    assert.equal(config.marketId, marketId);
    assert.equal(config.isResolved, false);
    assert.equal(config.protocolFeeBps.toNumber(), 50);
  });

  it("2. Underwrites prediction pool and generates Nectar yield shares & paired outcome tokens", async () => {
    await program.methods
      .depositAndUnderwrite(initialDeposit)
      .accounts({
        marketAccount: marketConfigPda,
        vaultEscrow: vaultEscrowPda,
        user: wallet.publicKey,
        userUnderlying: userUnderlyingAccount,
        nectarReserveVault: nectarReserveVault,
        nectarPoolReceiptSource: nectarPoolReceiptSource,
        nectarPoolAuthority: wallet.publicKey,
        vaultReceiptAccount: vaultReceiptAccount,
        yesTokenMint: yesMintKeypair.publicKey,
        noTokenMint: noMintKeypair.publicKey,
        userYesAccount: userYesAccount,
        userNoAccount: userNoAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const userYesToken = await getAccount(provider.connection, userYesAccount);
    const userNoToken = await getAccount(provider.connection, userNoAccount);

    assert.equal(userYesToken.amount.toString(), initialDeposit.toString());
    assert.equal(userNoToken.amount.toString(), initialDeposit.toString());
  });

  it("3. Resolves market via Dominion Oracle with 8% compound yield index expansion", async () => {
    // 8% yield accrued: 1.0 -> 1.08
    const baseIndex = new anchor.BN("1000000000000"); // 1e12
    const finalYieldIndex = new anchor.BN("1080000000000"); // 1.08e12

    await program.methods
      .resolveMarket({ yes: {} }, finalYieldIndex)
      .accounts({
        marketAccount: marketConfigPda,
        oracle: wallet.publicKey,
      })
      .rpc();

    const marketState = await program.account.marketConfig.fetch(marketConfigPda);
    assert.isTrue(marketState.isResolved);
    assert.deepEqual(marketState.winningOutcome, { yes: {} });
    assert.equal(marketState.settledYieldPerShare.toString(), finalYieldIndex.toString());
  });

  it("4. Settles winning YES tokens and redeems principal plus Nectar yield with protocol fee deducted", async () => {
    const balanceBefore = await getAccount(provider.connection, userUnderlyingAccount);

    await program.methods
      .settleAndClaimYield(initialDeposit)
      .accounts({
        marketAccount: marketConfigPda,
        vaultEscrow: vaultEscrowPda,
        user: wallet.publicKey,
        winningMint: yesMintKeypair.publicKey,
        userWinningTokenAccount: userYesAccount,
        vaultUnderlyingReserve: vaultUnderlyingReserve,
        userUnderlying: userUnderlyingAccount,
        protocolFeeReceiver: protocolFeeReceiver,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const balanceAfter = await getAccount(provider.connection, userUnderlyingAccount);
    const payoutDiff = new anchor.BN(balanceAfter.amount.toString()).sub(
      new anchor.BN(balanceBefore.amount.toString())
    );

    // Expected:
    // Principal: 1_000_000_000
    // Yield (8%): 80_000_000
    // Gross: 1_080_000_000
    // Fee (0.50%): 5_400_000
    // Net: 1_074_600_000
    assert.equal(payoutDiff.toString(), "1074600000");

    const yesAccountPost = await getAccount(provider.connection, userYesAccount);
    assert.equal(yesAccountPost.amount.toString(), "0");
  });

  it("5. Rejects settlement on losing NO token outcome", async () => {
    try {
      await program.methods
        .settleAndClaimYield(initialDeposit)
        .accounts({
          marketAccount: marketConfigPda,
          vaultEscrow: vaultEscrowPda,
          user: wallet.publicKey,
          winningMint: noMintKeypair.publicKey,
          userWinningTokenAccount: userNoAccount,
          vaultUnderlyingReserve: vaultUnderlyingReserve,
          userUnderlying: userUnderlyingAccount,
          protocolFeeReceiver: protocolFeeReceiver,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();
      assert.fail("Should have thrown error on losing token settlement");
    } catch (err: any) {
      assert.include(err.toString(), "InvalidWinningAccount");
    }
  });
});