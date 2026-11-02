import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { DominionNectarVault } from "../target/types/dominion_nectar_vault";

async function runKeeper() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.DominionNectarVault as Program<DominionNectarVault>;

  console.log("Dominion Keeper Daemon active. Monitoring markets...");

  setInterval(async () => {
    const markets = await program.account.marketPool.all([
      { memcmp: { offset: 8 + 32 + 32 + 32 + 32 + 32 + 32 + 8 + 8 + 8 + 16 + 16 + 8 + 8, bytes: anchor.utils.bytes.bs58.encode(Buffer.from([0])) } }
    ]);

    const now = Math.floor(Date.now() / 1000);
    for (const m of markets) {
      if (m.account.maturityTs.toNumber() <= now) {
        console.log(`Settling expired market: ${m.publicKey.toBase58()}`);
        await program.methods
          .settleMarket()
          .accounts({
            market: m.publicKey,
            nectarVault: m.account.nectarVault,
            keeper: provider.wallet.publicKey,
          })
          .rpc();
        console.log(`Successfully settled: ${m.publicKey.toBase58()}`);
      }
    }
  }, 5000);
}

runKeeper().catch(console.error);