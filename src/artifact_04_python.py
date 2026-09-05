"""
Autonomous Keeper Service for NectarFi x Dominion Market
Monitors maturity timestamps and health ratios to trigger on-chain settlement.
"""

import asyncio
import logging
import os
import time
from typing import Dict, Any
from solana.rpc.async_api import AsyncClient
from solders.pubkey import Pubkey
from solders.keypair import Keypair
from solders.instruction import Instruction, AccountMeta
from solders.transaction import Transaction

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")

PROGRAM_ID = Pubkey.from_string("NectDom1111111111111111111111111111111111111")
RPC_ENDPOINT = os.getenv("SOLANA_RPC_URL", "http://127.0.0.1:8899")

class NectarDominionKeeper:
    def __init__(self, keypair_path: str):
        self.client = AsyncClient(RPC_ENDPOINT)
        with open(keypair_path, "rb") as f:
            self.keeper_keypair = Keypair.from_bytes(f.read())
        logging.info(f"Keeper initialized with pubkey: {self.keeper_keypair.pubkey()}")

    async def scan_and_settle(self, vaults: list[Pubkey]):
        current_ts = int(time.time())
        for vault_pubkey in vaults:
            account_info = await self.client.get_account_info(vault_pubkey)
            if not account_info or not account_info.value:
                continue

            data = account_info.value.data
            # Unpack Vault state layout:
            # 8 bytes disc + 32 auth + 32 u_mint + 32 s_mint + 32 j_mint + 32 v_res + 32 dom_mkt + 32 dom_note
            # + 8 apy + 8 ratio + 8 maturity_ts + 8 cap + 8 sen_dep + 8 jun_dep + 8 sen_settled + 1 is_settled
            offset = 8 + (32 * 7) + 8 + 8
            maturity_ts = int.from_bytes(data[offset:offset+8], byteorder="little", signed=True)
            is_settled = bool(data[offset + 40])

            logging.info(f"Vault {vault_pubkey}: Maturity={maturity_ts}, Settled={is_settled}, Now={current_ts}")

            if current_ts >= maturity_ts and not is_settled:
                logging.info(f"Triggering atomic maturity settlement for {vault_pubkey}...")
                await self.execute_settlement(vault_pubkey)

    async def execute_settlement(self, vault_pubkey: Pubkey):
        # Discriminator for 'settle_maturity'
        # sha256("global:settle_maturity")[:8]
        settle_discriminator = bytes([231, 142, 114, 18, 121, 239, 131, 189])
        
        # In a production context, proceeds are fetched from Dominion Escrow
        simulated_proceeds = (100_000 * 10**6).to_bytes(8, byteorder="little")
        ix_data = settle_discriminator + simulated_proceeds

        ix = Instruction(
            program_id=PROGRAM_ID,
            data=ix_data,
            accounts=[
                AccountMeta(pubkey=self.keeper_keypair.pubkey(), is_signer=True, is_writable=True),
                AccountMeta(pubkey=vault_pubkey, is_signer=False, is_writable=True),
            ]
        )

        recent_blockhash = (await self.client.get_latest_blockhash()).value.blockhash
        tx = Transaction([ix], self.keeper_keypair.pubkey())
        tx.sign([self.keeper_keypair], recent_blockhash)

        res = await self.client.send_raw_transaction(bytes(tx))
        logging.info(f"Settlement tx dispatched: {res}")

    async def run(self):
        while True:
            try:
                # Poll vaults
                demo_vault = Pubkey.find_program_address([b"vault"], PROGRAM_ID)[0]
                await self.scan_and_settle([demo_vault])
            except Exception as e:
                logging.error(f"Keeper loop encountered an error: {e}")
            await asyncio.sleep(5)

if __name__ == "__main__":
    # Start bot
    # keeper = NectarDominionKeeper("keeper-keypair.json")
    # asyncio.run(keeper.run())
    pass