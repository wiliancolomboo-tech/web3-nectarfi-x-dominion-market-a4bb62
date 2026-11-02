import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { NectarDominion } from "../target/types/nectar_dominion";

async function runHarvestKeeper() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.NectarDominion as Program<NectarDominion>;

  console.log("Starting Dominion x NectarFi Yield Crank Keeper...");

  const markets = await program.account.marketState.all();
  for (const market of markets) {
    try {
      console.log(`Checking market ID: ${market.account.marketId.toString()}`);
      
      const tx = await program.methods
        .harvestYield()
        .accounts({
          market: market.publicKey,
          vaultPda: anchor.web3.PublicKey.findProgramAddressSync(
            [Buffer.from("vault"), market.account.marketId.toArrayLike(Buffer, "le", 8)],
            program.programId
          )[0],
          vaultNectarShares: market.account.nectarShareMint,
          protocolTreasury: market.account.authority,
          nectarProgram: program.programId,
          nectarVault: market.account.nectarVault,
          tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
        })
        .rpc();

      console.log(`Harvested surplus successfully. Signature: ${tx}`);
    } catch (err: any) {
      console.log(`No harvestable surplus for market ${market.account.marketId}: ${err.message}`);
    }
  }
}

runHarvestKeeper().catch(console.error);