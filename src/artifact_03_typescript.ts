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

describe("NectarFi x Dominion Market: Tranche & Settlement Tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.NectarDominion as Program<NectarDominion>;

  let underlyingMint: anchor.web3.PublicKey;
  let dominionNoteMint: anchor.web3.PublicKey;
  const dominionMarket = anchor.web3.Keypair.generate();

  let vaultPda: anchor.web3.PublicKey;
  let vaultBump: number;
  let reservePda: anchor.web3.PublicKey;
  let seniorMintPda: anchor.web3.PublicKey;
  let juniorMintPda: anchor.web3.PublicKey;

  const seniorUser = anchor.web3.Keypair.generate();
  const juniorUser = anchor.web3.Keypair.generate();
  const keeperUser = anchor.web3.Keypair.generate();

  let seniorUserUnderlying: anchor.web3.PublicKey;
  let seniorUserShare: anchor.web3.PublicKey;

  let juniorUserUnderlying: anchor.web3.PublicKey;
  let juniorUserShare: anchor.web3.PublicKey;

  let vaultNoteCustody: anchor.web3.PublicKey;
  let dominionEscrow: anchor.web3.PublicKey;

  const FIXED_SENIOR_APY_BPS = new anchor.BN(800); // 8%
  const JUNIOR_RATIO_BPS = new anchor.BN(2000); // 20% protection buffer
  const MAX_CAPACITY = new anchor.BN(10_000_000 * 1e6); // 10M USDC
  let maturityTimestamp: anchor.BN;

  before(async () => {
    // AirDrop SOL for signatures
    for (const kp of [seniorUser, juniorUser, keeperUser]) {
      const sig = await provider.connection.requestAirdrop(kp.publicKey, 5 * anchor.web3.LAMPORTS_PER_SOL);
      await provider.connection.confirmTransaction(sig);
    }

    // Create underlying mock USDC mint
    underlyingMint = await createMint(
      provider.connection,
      (provider.wallet as any).payer,
      provider.wallet.publicKey,
      null,
      6
    );

    // Create mock Dominion Note mint
    dominionNoteMint = await createMint(
      provider.connection,
      (provider.wallet as any).payer,
      provider.wallet.publicKey,
      null,
      6
    );

    // Derive PDAs
    [vaultPda, vaultBump] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), dominionMarket.publicKey.toBuffer()],
      program.programId
    );

    [reservePda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault_reserve"), vaultPda.toBuffer()],
      program.programId
    );

    [seniorMintPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("senior_mint"), vaultPda.toBuffer()],
      program.programId
    );

    [juniorMintPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("junior_mint"), vaultPda.toBuffer()],
      program.programId
    );

    // Setup Token Accounts
    seniorUserUnderlying = await createAccount(
      provider.connection,
      (provider.wallet as any).payer,
      underlyingMint,
      seniorUser.publicKey
    );

    juniorUserUnderlying = await createAccount(
      provider.connection,
      (provider.wallet as any).payer,
      underlyingMint,
      juniorUser.publicKey
    );

    vaultNoteCustody = await createAccount(
      provider.connection,
      (provider.wallet as any).payer,
      dominionNoteMint,
      vaultPda
    );

    dominionEscrow = await createAccount(
      provider.connection,
      (provider.wallet as any).payer,
      underlyingMint,
      dominionMarket.publicKey
    );

    // Fund user balances: Junior gets 50,000, Senior gets 100,000
    await mintTo(provider.connection, (provider.wallet as any).payer, underlyingMint, seniorUserUnderlying, (provider.wallet as any).payer, 100_000 * 1e6);
    await mintTo(provider.connection, (provider.wallet as any).payer, underlyingMint, juniorUserUnderlying, (provider.wallet as any).payer, 50_000 * 1e6);
  });

  it("1. Initializes Vault linked to Dominion Market Note", async () => {
    const currentBlockTime = (await provider.connection.getBlockTime(await provider.connection.getSlot())) || Math.floor(Date.now() / 1000);
    maturityTimestamp = new anchor.BN(currentBlockTime + 4); // 4 seconds for fast local test

    await program.methods
      .initializeVault(
        FIXED_SENIOR_APY_BPS,
        JUNIOR_RATIO_BPS,
        maturityTimestamp,
        MAX_CAPACITY
      )
      .accounts({
        authority: provider.wallet.publicKey,
        underlyingMint,
        vault: vaultPda,
        vaultUnderlyingReserve: reservePda,
        seniorMint: seniorMintPda,
        juniorMint: juniorMintPda,
        dominionMarket: dominionMarket.publicKey,
        dominionNoteMint,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const vaultAccount = await program.account.vault.fetch(vaultPda);
    expect(vaultAccount.seniorFixedApyBps.toNumber()).to.equal(800);
    expect(vaultAccount.isSettled).to.be.false;
  });

  it("2. Prevents Senior deposit without required Junior first-loss protection buffer", async () => {
    seniorUserShare = await createAccount(
      provider.connection,
      seniorUser,
      seniorMintPda,
      seniorUser.publicKey
    );

    try {
      await program.methods
        .depositSenior(new anchor.BN(10_000 * 1e6))
        .accounts({
          depositor: seniorUser.publicKey,
          vault: vaultPda,
          vaultUnderlyingReserve: reservePda,
          seniorMint: seniorMintPda,
          userUnderlyingAccount: seniorUserUnderlying,
          userSeniorAccount: seniorUserShare,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([seniorUser])
        .rpc();
      assert.fail("Should have failed due to missing junior buffer");
    } catch (err: any) {
      expect(err.toString()).to.contain("InsufficientJuniorProtectionBuffer");
    }
  });

  it("3. Successfully deposits Junior first-loss tranche capital", async () => {
    juniorUserShare = await createAccount(
      provider.connection,
      juniorUser,
      juniorMintPda,
      juniorUser.publicKey
    );

    const depositAmount = new anchor.BN(20_000 * 1e6);

    await program.methods
      .depositJunior(depositAmount)
      .accounts({
        depositor: juniorUser.publicKey,
        vault: vaultPda,
        vaultUnderlyingReserve: reservePda,
        juniorMint: juniorMintPda,
        userUnderlyingAccount: juniorUserUnderlying,
        userJuniorAccount: juniorUserShare,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([juniorUser])
      .rpc();

    const jnrShareAccount = await getAccount(provider.connection, juniorUserShare);
    expect(Number(jnrShareAccount.amount)).to.equal(20_000 * 1e6);
  });

  it("4. Accepts Senior deposits once safety buffer is established", async () => {
    const depositSeniorAmount = new anchor.BN(80_000 * 1e6);

    await program.methods
      .depositSenior(depositSeniorAmount)
      .accounts({
        depositor: seniorUser.publicKey,
        vault: vaultPda,
        vaultUnderlyingReserve: reservePda,
        seniorMint: seniorMintPda,
        userUnderlyingAccount: seniorUserUnderlying,
        userSeniorAccount: seniorUserShare,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([seniorUser])
      .rpc();

    const secShareAccount = await getAccount(provider.connection, seniorUserShare);
    expect(Number(secShareAccount.amount)).to.equal(80_000 * 1e6);
  });

  it("5. Settles vault post-maturity and enforces senior priority waterfall", async () => {
    // Wait for maturity timestamp to pass
    await new Promise((resolve) => setTimeout(resolve, 5000));

    // Simulate Dominion Note settlement proceeds returning into Reserve:
    // 80k senior + 8% yield (6.4k) + 20k junior + 5k excess return = 111,400 total
    const totalProceeds = 111_400 * 1e6;
    await mintTo(
      provider.connection,
      (provider.wallet as any).payer,
      underlyingMint,
      reservePda,
      (provider.wallet as any).payer,
      11_400 * 1e6 // Adding the profit portion directly to reserve
    );

    await program.methods
      .settleMaturity(new anchor.BN(totalProceeds))
      .accounts({
        crankKeeper: keeperUser.publicKey,
        vault: vaultPda,
        vaultUnderlyingReserve: reservePda,
        vaultNoteCustody,
        dominionNoteMint,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([keeperUser])
      .rpc();

    const vaultAccount = await program.account.vault.fetch(vaultPda);
    expect(vaultAccount.isSettled).to.be.true;
    expect(vaultAccount.seniorYieldSettled.toNumber()).to.equal(6_400 * 1e6);
  });

  it("6. Senior redeems full principal + fixed guaranteed yield", async () => {
    const seniorBurnAmount = new anchor.BN(80_000 * 1e6);

    const initialUnderlying = await getAccount(provider.connection, seniorUserUnderlying);

    await program.methods
      .redeemSenior(seniorBurnAmount)
      .accounts({
        redeemer: seniorUser.publicKey,
        vault: vaultPda,
        vaultUnderlyingReserve: reservePda,
        seniorMint: seniorMintPda,
        userSeniorAccount: seniorUserShare,
        userUnderlyingAccount: seniorUserUnderlying,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([seniorUser])
      .rpc();

    const finalUnderlying = await getAccount(provider.connection, seniorUserUnderlying);
    const balanceDiff = Number(finalUnderlying.amount) - Number(initialUnderlying.amount);
    expect(balanceDiff).to.equal(86_400 * 1e6); // 80k principal + 6.4k coupon
  });

  it("7. Junior redeems residual alpha profit pool", async () => {
    const juniorBurnAmount = new anchor.BN(20_000 * 1e6);
    const initialUnderlying = await getAccount(provider.connection, juniorUserUnderlying);

    await program.methods
      .redeemJunior(juniorBurnAmount)
      .accounts({
        redeemer: juniorUser.publicKey,
        vault: vaultPda,
        vaultUnderlyingReserve: reservePda,
        juniorMint: juniorMintPda,
        seniorMint: seniorMintPda,
        userJuniorAccount: juniorUserShare,
        userUnderlyingAccount: juniorUserUnderlying,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([juniorUser])
      .rpc();

    const finalUnderlying = await getAccount(provider.connection, juniorUserUnderlying);
    const balanceDiff = Number(finalUnderlying.amount) - Number(initialUnderlying.amount);
    expect(balanceDiff).to.equal(25_000 * 1e6); // 20k principal + 5k excess carry
  });
});