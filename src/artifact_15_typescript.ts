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
import { assert, expect } from "chai";
import { Keypair, LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";

describe("NectarFi x Dominion Market Full E2E Test Pipeline", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.NectarDominion as Program<NectarDominion>;
  const authority = (provider.wallet as anchor.Wallet).payer;

  let baseMint: PublicKey;
  let collateralMint: PublicKey; // nctSOL mock
  let marketPda: PublicKey;
  let ptMintPda: PublicKey;
  let ytMintPda: PublicKey;
  let collateralVaultPda: PublicKey;
  let baseVaultPda: PublicKey;
  let ptVaultPda: PublicKey;

  const scalarRoot = new anchor.BN(10_000);
  let maturityTimestamp: anchor.BN;

  let alice = Keypair.generate();
  let aliceCollateralAccount: PublicKey;
  let alicePtAccount: PublicKey;
  let aliceYtAccount: PublicKey;
  let alicePositionPda: PublicKey;

  before(async () => {
    // 1. Airdrop funds to test participants
    const tx = await provider.connection.requestAirdrop(alice.publicKey, 10 * LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(tx, "confirmed");

    // 2. Setup mock SPL tokens
    baseMint = await createMint(provider.connection, authority, authority.publicKey, null, 9);
    collateralMint = await createMint(provider.connection, authority, authority.publicKey, null, 9);

    // 3. Set maturity 30 seconds into the future for deterministic testing
    const currentBlockTime = (await provider.connection.getBlockTime(await provider.connection.getSlot())) || Math.floor(Date.now() / 1000);
    maturityTimestamp = new anchor.BN(currentBlockTime + 15);

    // 4. Derive PDAs
    const maturityLeBytes = maturityTimestamp.toArrayLike(Buffer, "le", 8);

    [marketPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("dominion_market"), collateralMint.toBuffer(), maturityLeBytes],
      program.programId
    );

    [ptMintPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("pt_mint"), marketPda.toBuffer()],
      program.programId
    );

    [ytMintPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("yt_mint"), marketPda.toBuffer()],
      program.programId
    );

    [collateralVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), marketPda.toBuffer(), collateralMint.toBuffer()],
      program.programId
    );

    [baseVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), marketPda.toBuffer(), baseMint.toBuffer()],
      program.programId
    );

    [ptVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), marketPda.toBuffer(), ptMintPda.toBuffer()],
      program.programId
    );

    [alicePositionPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("position"), marketPda.toBuffer(), alice.publicKey.toBuffer()],
      program.programId
    );

    // Initialize Token Accounts for Alice
    aliceCollateralAccount = await createAccount(provider.connection, alice, collateralMint, alice.publicKey);
    alicePtAccount = await createAccount(provider.connection, alice, ptMintPda, alice.publicKey);
    aliceYtAccount = await createAccount(provider.connection, alice, ytMintPda, alice.publicKey);

    // Mint test collateral (100 nctSOL) to Alice
    await mintTo(provider.connection, authority, collateralMint, aliceCollateralAccount, authority, 100_000_000_000);
  });

  it("Initializes the Dominion Yield-Stripping Market", async () => {
    await program.methods
      .initializeMarket(maturityTimestamp, scalarRoot)
      .accounts({
        authority: authority.publicKey,
        baseMint: baseMint,
        collateralMint: collateralMint,
        marketState: marketPda,
        ptMint: ptMintPda,
        ytMint: ytMintPda,
        collateralVault: collateralVaultPda,
        baseVault: baseVaultPda,
        ptVault: ptVaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const marketStateAccount = await program.account.marketState.fetch(marketPda);
    expect(marketStateAccount.authority.toBase58()).to.equal(authority.publicKey.toBase58());
    expect(marketStateAccount.collateralMint.toBase58()).to.equal(collateralMint.toBase58());
    expect(marketStateAccount.maturityTimestamp.toNumber()).to.equal(maturityTimestamp.toNumber());
    expect(marketStateAccount.totalCollateralLocked.toNumber()).to.equal(0);
  });

  it("Splits nctSOL collateral into 1:1 Principal Tokens (PT) and Yield Tokens (YT)", async () => {
    const depositAmount = new anchor.BN(20_000_000_000); // 20 nctSOL

    await program.methods
      .tokenizeYield(depositAmount)
      .accounts({
        user: alice.publicKey,
        marketState: marketPda,
        collateralVault: collateralVaultPda,
        ptMint: ptMintPda,
        ytMint: ytMintPda,
        userCollateral: aliceCollateralAccount,
        userPt: alicePtAccount,
        userYt: aliceYtAccount,
        userPosition: alicePositionPda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([alice])
      .rpc();

    // Verify token account balances
    const alicePt = await getAccount(provider.connection, alicePtAccount);
    const aliceYt = await getAccount(provider.connection, aliceYtAccount);
    const vaultCollateral = await getAccount(provider.connection, collateralVaultPda);

    expect(Number(alicePt.amount)).to.equal(20_000_000_000);
    expect(Number(aliceYt.amount)).to.equal(20_000_000_000);
    expect(Number(vaultCollateral.amount)).to.equal(20_000_000_000);

    const position = await program.account.userYieldPosition.fetch(alicePositionPda);
    expect(position.ytBalance.toNumber()).to.equal(20_000_000_000);
  });

  it("Guards against premature Principal Token (PT) redemption before maturity", async () => {
    const redeemAmount = new anchor.BN(5_000_000_000);

    try {
      await program.methods
        .redeemPt(redeemAmount)
        .accounts({
          user: alice.publicKey,
          marketState: marketPda,
          ptMint: ptMintPda,
          collateralVault: collateralVaultPda,
          userPt: alicePtAccount,
          userCollateralReceive: aliceCollateralAccount,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([alice])
        .rpc();
      assert.fail("Redemption should have failed prior to maturity.");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("MarketNotMatured");
    }
  });

  it("Executes full 1:1 redemption of PT after maturity timestamp elapsed", async () => {
    // Wait for the maturity time window (16 seconds)
    console.log("    ...Sleeping until term maturity...");
    await new Promise((resolve) => setTimeout(resolve, 16000));

    const redeemAmount = new anchor.BN(20_000_000_000);

    await program.methods
      .redeemPt(redeemAmount)
      .accounts({
        user: alice.publicKey,
        marketState: marketPda,
        ptMint: ptMintPda,
        collateralVault: collateralVaultPda,
        userPt: alicePtAccount,
        userCollateralReceive: aliceCollateralAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([alice])
      .rpc();

    const aliceCollateral = await getAccount(provider.connection, aliceCollateralAccount);
    const alicePt = await getAccount(provider.connection, alicePtAccount);

    expect(Number(alicePt.amount)).to.equal(0);
    expect(Number(aliceCollateral.amount)).to.equal(100_000_000_000); // Fully recovered
  });
});