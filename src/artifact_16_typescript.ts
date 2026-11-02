import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { NectarDominionVault } from "../target/types/nectar_dominion_vault";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.NectarDominionVault as Program<NectarDominionVault>;

  const marketId = new anchor.BN(1001);
  const [marketVaultPda] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("dominion_market_vault"), marketId.toArrayLike(Buffer, "le", 8)],
    program.programId
  );

  console.log(`[+] Initializing Autonomous Keeper for Market: ${marketVaultPda.toBase58()}`);

  setInterval(async () => {
    try {
      const vaultState = await program.account.marketVault.fetch(marketVaultPda);
      const vaultCollateralBalance = await provider.connection.getTokenAccountBalance(vaultState.collateralVault);

      const currentBalance = new anchor.BN(vaultCollateralBalance.value.amount);
      const basePrincipal = vaultState.totalCollateralDeposited;

      if (currentBalance.gt(basePrincipal)) {
        console.log(`[!] Unharvested yield detected: ${currentBalance.sub(basePrincipal).toString()} units. Cranking...`);
        const tx = await program.methods
          .harvestAndCompound()
          .accounts({
            keeper: provider.wallet.publicKey,
            marketVault: marketVaultPda,
            collateralVault: vaultState.collateralVault,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc();
        console.log(`[✓] Harvest successful. Tx Signature: ${tx}`);
      } else {
        console.log(`[-] No excess yield accrued. Sleeping...`);
      }
    } catch (err) {
      console.error(`[x] Keeper error:`, err);
    }
  }, 10000); // Poll every 10 seconds
}

main().catch(console.error);