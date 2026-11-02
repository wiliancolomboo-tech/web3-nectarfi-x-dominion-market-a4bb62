// Initializing protocol config via script
import * as anchor from "@coral-xyz/anchor";

const provider = anchor.AnchorProvider.env();
const program = anchor.workspace.DominionNectarVault;

async function setup() {
  const [configPda] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    program.programId
  );

  const tx = await program.methods
    .initializeProtocol(500) // 500 BPS = 5%
    .accounts({
      config: configPda,
      authority: provider.wallet.publicKey,
      treasury: provider.wallet.publicKey,
      systemProgram: anchor.web3.SystemProgram.programId,
    })
    .rpc();

  console.log("Protocol initialized successfully. Signature:", tx);
}

setup();