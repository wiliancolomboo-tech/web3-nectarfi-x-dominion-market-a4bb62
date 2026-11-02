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
import { assert } from "chai";

describe("nectar-dominion core protocol test matrix", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.NectarDominion as Program<NectarDominion>;
  const connection = provider.connection;

  let collateralMint: anchor.web3.PublicKey;
  let nectarShareMint: anchor.web3.PublicKey;
  let resolutionAuthority: anchor.web3.Keypair;
  let traderUser: anchor.web3.Keypair;

  let marketPda: anchor.web3.PublicKey;
  let marketBump: number;
  let outcomeMintYes: anchor.web3.PublicKey;
  let outcomeMintNo: anchor.web3.PublicKey;
  let vaultNecShares: anchor.web3.PublicKey;
  let collateralEscrow: anchor.web3.PublicKey;

  let userCollateralAccount: anchor.web3.PublicKey;
  let userOutcomeYes: anchor.web3.PublicKey;
  let userOutcomeNo: anchor.web3.PublicKey;

  const INITIAL_DEPOSIT = new anchor.BN(500_000_000); // 500 USDC (6 decimals)
  let targetResolutionSlot: anchor.BN;

  before(async () => {
    resolutionAuthority = anchor.web3.Keypair.generate();
    traderUser = anchor.web3.Keypair.generate();

    // Airdrop SOL to test actors
    for (const actor of [resolutionAuthority, traderUser]) {
      const airdropSig = await connection.requestAirdrop(
        actor.publicKey,
        5 * anchor.web3.LAMPORTS_PER_SOL
      );
      await connection.confirmTransaction(airdropSig);
    }

    // Initialize mock collateral (USDC)
    collateralMint = await createMint(
      connection,
      (provider.wallet as any).payer,
      provider.wallet.publicKey,
      null,
      6
    );

    // Derive PDA addresses
    [marketPda, marketBump] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("market"), collateralMint.toBuffer()],
      program.programId
    );

    [outcomeMintYes] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("outcome_yes"), marketPda.toBuffer()],
      program.programId
    );

    [outcomeMintNo] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("outcome_no"), marketPda.toBuffer()],
      program.programId
    );

    [vaultNecShares] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("nec_shares"), marketPda.toBuffer()],
      program.programId
    );

    [collateralEscrow] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("collateral_escrow"), marketPda.toBuffer()],
      program.programId
    );

    // Create mock Nectar Vault Share Mint authorized by the market PDA
    nectarShareMint = await createMint(
      connection,
      (provider.wallet as any).payer,
      marketPda,
      null,
      6
    );

    // Setup User token accounts
    userCollateralAccount = await createAccount(
      connection,
      traderUser,
      collateralMint,
      traderUser.publicKey
    );

    userOutcomeYes = await createAccount(
      connection,
      traderUser,
      outcomeMintYes,
      traderUser.publicKey
    );

    userOutcomeNo = await createAccount(
      connection,
      traderUser,
      outcomeMintNo,
      traderUser.publicKey
    );

    // Mint USDC collateral into user wallet
    await mintTo(
      connection,
      (provider.wallet as any).payer,
      collateralMint,
      userCollateralAccount,
      provider.wallet.publicKey,
      1_000_000_000 // 1,000 USDC
    );
  });

  it("1. Successfully initializes the Dominion Market linked to NectarFi", async () => {
    const currentSlot = await connection.getSlot();
    targetResolutionSlot = new anchor.BN(currentSlot + 25);

    const tx = await program.methods
      .initializeMarket(targetResolutionSlot)
      .accounts({
        market: marketPda,
        collateralMint: collateralMint,
        nectarShareMint: nectarShareMint,
        outcomeMintYes: outcomeMintYes,
        outcomeMintNo: outcomeMintNo,
        vaultNecShares: vaultNecShares,
        collateralEscrow: collateralEscrow,
        resolutionAuthority: resolutionAuthority.publicKey,
        payer: provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    assert.isString(tx);
    const marketState = await program.account.marketConfig.fetch(marketPda);
    assert.equal(
      marketState.resolutionAuthority.toBase58(),
      resolutionAuthority.publicKey.toBase58()
    );
    assert.equal(marketState.outcome, 0); // Unresolved
    assert.equal(marketState.totalCollateralLocked.toNumber(), 0);
  });

  it("2. Mints paired YES/NO shares and routes capital into NectarFi yield escrow", async () => {
    const tx = await program.methods
      .mintConditionalShares(INITIAL_DEPOSIT)
      .accounts({
        market: marketPda,
        collateralEscrow: collateralEscrow,
        vaultNecShares: vaultNecShares,
        nectarShareMint: nectarShareMint,
        outcomeMintYes: outcomeMintYes,
        outcomeMintNo: outcomeMintNo,
        userCollateralAccount: userCollateralAccount,
        userOutcomeYes: userOutcomeYes,
        userOutcomeNo: userOutcomeNo,
        user: traderUser.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([traderUser])
      .rpc();

    assert.isString(tx);

    const yesBalance = await getAccount(connection, userOutcomeYes);
    const noBalance = await getAccount(connection, userOutcomeNo);
    const necVaultBalance = await getAccount(connection, vaultNecShares);

    assert.equal(yesBalance.amount.toString(), INITIAL_DEPOSIT.toString());
    assert.equal(noBalance.amount.toString(), INITIAL_DEPOSIT.toString());
    assert.equal(necVaultBalance.amount.toString(), INITIAL_DEPOSIT.toString());

    const marketState = await program.account.marketConfig.fetch(marketPda);
    assert.equal(
      marketState.totalCollateralLocked.toString(),
      INITIAL_DEPOSIT.toString()
    );
  });

  it("3. Prevents premature market settlement prior to resolution slot", async () => {
    try {
      await program.methods
        .settleMarket(1) // YES outcome
        .accounts({
          market: marketPda,
          resolutionAuthority: resolutionAuthority.publicKey,
        })
        .signers([resolutionAuthority])
        .rpc();
      assert.fail("Should have thrown MarketNotResolved error");
    } catch (err: any) {
      assert.include(err.toString(), "MarketNotResolved");
    }
  });

  it("4. Settles the market with valid authority after slot threshold is met", async () => {
    // Fast-forward slots by spinning dummy slot updates
    let currentSlot = await connection.getSlot();
    while (currentSlot <= targetResolutionSlot.toNumber()) {
      await new Promise((resolve) => setTimeout(resolve, 500));
      currentSlot = await connection.getSlot();
    }

    const tx = await program.methods
      .settleMarket(1) // Outcome: YES wins
      .accounts({
        market: marketPda,
        resolutionAuthority: resolutionAuthority.publicKey,
      })
      .signers([resolutionAuthority])
      .rpc();

    assert.isString(tx);
    const marketState = await program.account.marketConfig.fetch(marketPda);
    assert.equal(marketState.outcome, 1); // Outcome::Yes
  });

  it("5. Redeems winning outcome tokens for base collateral + realized yield", async () => {
    const preRedeemCollateral = await getAccount(
      connection,
      userCollateralAccount
    );

    const tx = await program.methods
      .redeemSettledShares(INITIAL_DEPOSIT)
      .accounts({
        market: marketPda,
        collateralEscrow: collateralEscrow,
        userCollateralAccount: userCollateralAccount,
        outcomeMintYes: outcomeMintYes,
        outcomeMintNo: outcomeMintNo,
        userOutcomeAccount: userOutcomeYes,
        user: traderUser.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([traderUser])
      .rpc();

    assert.isString(tx);

    const postRedeemCollateral = await getAccount(
      connection,
      userCollateralAccount
    );
    const postYesBalance = await getAccount(connection, userOutcomeYes);

    assert.equal(postYesBalance.amount.toString(), "0");
    assert.equal(
      BigInt(postRedeemCollateral.amount) - BigInt(preRedeemCollateral.amount),
      BigInt(INITIAL_DEPOSIT.toString())
    );
  });
});