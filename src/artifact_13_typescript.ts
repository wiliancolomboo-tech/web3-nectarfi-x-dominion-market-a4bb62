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
import BN from "bn.js";

describe("NectarFi x Dominion Market QA Pipeline", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.NectarDominion as Program<NectarDominion>;
  const connection = provider.connection;
  const admin = (provider.wallet as anchor.Wallet).payer;
  const user = anchor.web3.Keypair.generate();

  let underlyingMint: anchor.web3.PublicKey;
  let ptMint: anchor.web3.PublicKey;
  let ytMint: anchor.web3.PublicKey;
  let dummyNectarVault = anchor.web3.Keypair.generate().publicKey;

  let marketPDA: anchor.web3.PublicKey;
  let marketBump: number;
  let vaultAuthorityPDA: anchor.web3.PublicKey;
  let vaultAuthorityBump: number;
  let vaultPDA: anchor.web3.PublicKey;
  let userLedgerPDA: anchor.web3.PublicKey;

  let userUnderlying: anchor.web3.PublicKey;
  let userPT: anchor.web3.PublicKey;
  let userYT: anchor.web3.PublicKey;

  const nowSec = Math.floor(Date.now() / 1000);
  const maturityTimestamp = new BN(nowSec + 4); // 4 seconds for fast local testing
  const DEPOSIT_AMOUNT = new BN(500_000_000); // 500 USDC (6 decimals)

  before(async () => {
    // Air-drop SOL to test user
    const sig = await connection.requestAirdrop(user.publicKey, 10 * anchor.web3.LAMPORTS_PER_SOL);
    await connection.confirmTransaction(sig, "confirmed");

    // Create Underlying Mint
    underlyingMint = await createMint(connection, admin, admin.publicKey, null, 6);

    // Derive PDAs
    [marketPDA, marketBump] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("dominion_market"),
        underlyingMint.toBuffer(),
        maturityTimestamp.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    [vaultAuthorityPDA, vaultAuthorityBump] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault_authority"), marketPDA.toBuffer()],
      program.programId
    );

    [vaultPDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("market_vault"), marketPDA.toBuffer()],
      program.programId
    );

    [userLedgerPDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("yield_ledger"), marketPDA.toBuffer(), user.publicKey.toBuffer()],
      program.programId
    );

    // Mint authority for PT & YT is delegated to vaultAuthorityPDA
    ptMint = await createMint(connection, admin, vaultAuthorityPDA, null, 6);
    ytMint = await createMint(connection, admin, vaultAuthorityPDA, null, 6);

    // Initialize user token accounts
    userUnderlying = await createAccount(connection, user, underlyingMint, user.publicKey);
    userPT = await createAccount(connection, user, ptMint, user.publicKey);
    userYT = await createAccount(connection, user, ytMint, user.publicKey);

    // Mint collateral to user
    await mintTo(connection, admin, underlyingMint, userUnderlying, admin.publicKey, 1_000_000_000);
  });

  it("1. Successfully initializes a Dominion fixed-tenor market", async () => {
    await program.methods
      .initializeMarket(maturityTimestamp, marketBump, vaultAuthorityBump)
      .accounts({
        admin: admin.publicKey,
        underlyingMint: underlyingMint,
        market: marketPDA,
        vaultAuthority: vaultAuthorityPDA,
        vault: vaultPDA,
        ptMint: ptMint,
        ytMint: ytMint,
        nectarVault: dummyNectarVault,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([admin])
      .rpc();

    const state = await program.account.marketState.fetch(marketPDA);
    expect(state.maturityTimestamp.toString()).to.equal(maturityTimestamp.toString());
    expect(state.totalPrincipal.toNumber()).to.equal(0);
    expect(state.underlyingMint.toBase58()).to.equal(underlyingMint.toBase58());
  });

  it("2. Deposits underlying collateral and splits into 1:1 PT and YT", async () => {
    await program.methods
      .depositAndSplit(DEPOSIT_AMOUNT)
      .accounts({
        user: user.publicKey,
        market: marketPDA,
        vaultAuthority: vaultAuthorityPDA,
        vault: vaultPDA,
        userUnderlying: userUnderlying,
        ptMint: ptMint,
        ytMint: ytMint,
        userPt: userPT,
        userYt: userYT,
        userLedger: userLedgerPDA,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([user])
      .rpc();

    const ptBalance = await getAccount(connection, userPT);
    const ytBalance = await getAccount(connection, userYT);
    const vaultBalance = await getAccount(connection, vaultPDA);

    assert.equal(ptBalance.amount.toString(), DEPOSIT_AMOUNT.toString());
    assert.equal(ytBalance.amount.toString(), DEPOSIT_AMOUNT.toString());
    assert.equal(vaultBalance.amount.toString(), DEPOSIT_AMOUNT.toString());
  });

  it("3. Fails redemption before maturity timestamp", async () => {
    try {
      await program.methods
        .redeemPt(DEPOSIT_AMOUNT)
        .accounts({
          user: user.publicKey,
          market: marketPDA,
          vaultAuthority: vaultAuthorityPDA,
          vault: vaultPDA,
          ptMint: ptMint,
          userPt: userPT,
          userUnderlying: userUnderlying,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([user])
        .rpc();
      assert.fail("Should have thrown MarketNotMatured");
    } catch (err: any) {
      expect(err.message).to.include("Market has not yet reached maturity");
    }
  });

  it("4. Redeems underlying 1:1 post maturity and burns PT", async () => {
    // Wait until maturity passes
    console.log("    Waiting 5 seconds for maturity timestamp...");
    await new Promise((resolve) => setTimeout(resolve, 5000));

    await program.methods
      .redeemPt(DEPOSIT_AMOUNT)
      .accounts({
        user: user.publicKey,
        market: marketPDA,
        vaultAuthority: vaultAuthorityPDA,
        vault: vaultPDA,
        ptMint: ptMint,
        userPt: userPT,
        userUnderlying: userUnderlying,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();

    const ptBalanceAfter = await getAccount(connection, userPT);
    const userUnderlyingAfter = await getAccount(connection, userUnderlying);

    assert.equal(ptBalanceAfter.amount.toString(), "0");
    assert.equal(userUnderlyingAfter.amount.toString(), "1000000000"); // Original 1,000 USDC balance recovered
  });
});