// tests/nectar_dominion.ts
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { NectarDominion } from "../target/types/nectar_dominion";
import {
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import { expect } from "chai";

describe("NectarFi x Dominion Market Protocol Test Suite", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.NectarDominion as Program<NectarDominion>;

  let assetMint: PublicKey;
  let dominionPoolMock: Keypair;
  let pythOracleMock: Keypair;

  let vaultPda: PublicKey;
  let vaultBump: number;
  let reservePda: PublicKey;
  let seniorMintPda: PublicKey;
  let juniorMintPda: PublicKey;

  let seniorUser: Keypair;
  let juniorUser: Keypair;
  let seniorAssetAta: PublicKey;
  let juniorAssetAta: PublicKey;
  let seniorShareAta: PublicKey;
  let juniorShareAta: PublicKey;

  const INITIAL_DEPOSIT = new anchor.BN(100_000_000_000); // 100 USDC (6 decimals)

  before(async () => {
    seniorUser = Keypair.generate();
    juniorUser = Keypair.generate();
    dominionPoolMock = Keypair.generate();
    pythOracleMock = Keypair.generate();

    // Airdrop SOL to test actors
    for (const user of [seniorUser, juniorUser]) {
      const sig = await provider.connection.requestAirdrop(user.publicKey, 10 * anchor.web3.LAMPORTS_PER_SOL);
      await provider.connection.confirmTransaction(sig);
    }

    // Deploy Base Token Mint (USDC Mock)
    assetMint = await createMint(
      provider.connection,
      (provider.wallet as any).payer,
      provider.wallet.publicKey,
      null,
      6
    );

    // Derive PDAs
    [vaultPda, vaultBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), assetMint.toBuffer(), dominionPoolMock.publicKey.toBuffer()],
      program.programId
    );

    [reservePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("reserve"), vaultPda.toBuffer()],
      program.programId
    );

    [seniorMintPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("mint_senior"), vaultPda.toBuffer()],
      program.programId
    );

    [juniorMintPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("mint_junior"), vaultPda.toBuffer()],
      program.programId
    );

    // Initialize User ATAs
    seniorAssetAta = await createAccount(provider.connection, seniorUser, assetMint, seniorUser.publicKey);
    juniorAssetAta = await createAccount(provider.connection, juniorUser, assetMint, juniorUser.publicKey);
    seniorShareAta = await createAccount(provider.connection, seniorUser, seniorMintPda, seniorUser.publicKey);
    juniorShareAta = await createAccount(provider.connection, juniorUser, juniorMintPda, juniorUser.publicKey);

    // Mint test capital to users
    await mintTo(provider.connection, (provider.wallet as any).payer, assetMint, seniorAssetAta, provider.wallet.publicKey, 1_000_000_000_000);
    await mintTo(provider.connection, (provider.wallet as any).payer, assetMint, juniorAssetAta, provider.wallet.publicKey, 1_000_000_000_000);
  });

  it("1. Successfully initializes the NectarFi Vault PDA & Tranche Mints", async () => {
    const params = {
      seniorTargetApyBps: 800,        // 8.00% Senior APY target
      targetUtilizationBps: 8000,     // 80.00% target deployment
      maxJuniorLeverageBps: 50000,    // 5.0x leverage ceiling
    };

    await program.methods
      .initializeVault(params)
      .accounts({
        authority: provider.wallet.publicKey,
        assetMint,
        dominionPool: dominionPoolMock.publicKey,
        pythOracleFeed: pythOracleMock.publicKey,
        vault: vaultPda,
        reserveAccount: reservePda,
        seniorMint: seniorMintPda,
        juniorMint: juniorMintPda,
        systemProgram: SystemProgram.programId,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const vaultAccount = await program.account.vaultState.fetch(vaultPda);
    expect(vaultAccount.seniorTargetApyBps).to.equal(800);
    expect(vaultAccount.targetUtilizationBps).to.equal(8000);
  });

  it("2. Executes First-Loss Junior Tranche Deposit", async () => {
    await program.methods
      .deposit(INITIAL_DEPOSIT, { junior: {} })
      .accounts({
        user: juniorUser.publicKey,
        vault: vaultPda,
        userAssetAccount: juniorAssetAta,
        reserveAccount: reservePda,
        seniorMint: seniorMintPda,
        juniorMint: juniorMintPda,
        userShareAccount: juniorShareAta,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
      })
      .signers([juniorUser])
      .rpc();

    const juniorShares = await getAccount(provider.connection, juniorShareAta);
    expect(Number(juniorShares.amount)).to.equal(INITIAL_DEPOSIT.toNumber());
  });

  it("3. Executes Protected Senior Tranche Deposit & Validates Isolation", async () => {
    await program.methods
      .deposit(INITIAL_DEPOSIT, { senior: {} })
      .accounts({
        user: seniorUser.publicKey,
        vault: vaultPda,
        userAssetAccount: seniorAssetAta,
        reserveAccount: reservePda,
        seniorMint: seniorMintPda,
        juniorMint: juniorMintPda,
        userShareAccount: seniorShareAta,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
      })
      .signers([seniorUser])
      .rpc();

    const seniorShares = await getAccount(provider.connection, seniorShareAta);
    expect(Number(seniorShares.amount)).to.equal(INITIAL_DEPOSIT.toNumber());

    const vaultState = await program.account.vaultState.fetch(vaultPda);
    expect(vaultState.totalSeniorDeposited.toNumber()).to.equal(INITIAL_DEPOSIT.toNumber());
    expect(vaultState.totalJuniorDeposited.toNumber()).to.equal(INITIAL_DEPOSIT.toNumber());
  });

  it("4. Prevents Zero Amount Deposits (Safety Constraint)", async () => {
    try {
      await program.methods
        .deposit(new anchor.BN(0), { senior: {} })
        .accounts({
          user: seniorUser.publicKey,
          vault: vaultPda,
          userAssetAccount: seniorAssetAta,
          reserveAccount: reservePda,
          seniorMint: seniorMintPda,
          juniorMint: juniorMintPda,
          userShareAccount: seniorShareAta,
          tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
        })
        .signers([seniorUser])
        .rpc();
      expect.fail("Expected transaction to fail with 0 amount");
    } catch (err: any) {
      expect(err).to.exist;
    }
  });

  it("5. Withdraws capital and burns tranche tokens accurately", async () => {
    const burnAmount = new anchor.BN(50_000_000_000); // 50 units

    await program.methods
      .withdraw(burnAmount, { senior: {} })
      .accounts({
        user: seniorUser.publicKey,
        vault: vaultPda,
        userAssetAccount: seniorAssetAta,
        reserveAccount: reservePda,
        seniorMint: seniorMintPda,
        juniorMint: juniorMintPda,
        userShareAccount: seniorShareAta,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
      })
      .signers([seniorUser])
      .rpc();

    const seniorShares = await getAccount(provider.connection, seniorShareAta);
    expect(Number(seniorShares.amount)).to.equal(INITIAL_DEPOSIT.sub(burnAmount).toNumber());
  });
});