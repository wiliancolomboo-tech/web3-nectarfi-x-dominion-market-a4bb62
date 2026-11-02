import asyncio
import json
import logging
from solana.rpc.async_api import AsyncClient
from solders.pubkey import Pubkey
from solders.keypair import Keypair
from solders.instruction import Instruction, AccountMeta
from solders.transaction import VersionedTransaction
from solders.message import MessageV0

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")

PROGRAM_ID = Pubkey.from_string("NectDom1111111111111111111111111111111111111")
RPC_ENDPOINT = "https://api.devnet.solana.com"
POLL_INTERVAL_SECONDS = 10

class DominionKeeper:
    def __init__(self, keypair_path: str):
        with open(keypair_path, "r") as f:
            secret = json.load(f)
        self.wallet = Keypair.from_bytes(bytes(secret))
        self.client = AsyncClient(RPC_ENDPOINT)
        logging.info(f"Keeper initialized with Operator Pubkey: {self.wallet.pubkey()}")

    async def get_market_accounts(self):
        """Scans Program Accounts for all active Dominion Term Markets"""
        resp = await self.client.get_program_accounts(PROGRAM_ID)
        return resp.value if resp else []

    async def execute_crank(self, market_pubkey: Pubkey, yield_escrow: Pubkey, simulated_index: int):
        """Builds and submits harvest_and_distribute transaction"""
        logging.info(f"Dispatching yield crank for Market: {market_pubkey}")

        # Scribe discriminator for `harvest_and_distribute`: sha256("global:harvest_and_distribute")[:8]
        # Precomputed discriminator: [181, 142, 219, 102, 114, 18, 146, 213]
        discriminator = bytes([181, 142, 219, 102, 114, 18, 146, 213])
        rate_bytes = simulated_index.to_bytes(16, byteorder='little')
        instruction_data = discriminator + rate_bytes

        accounts = [
            AccountMeta(pubkey=self.wallet.pubkey(), is_signer=True, is_writable=False),
            AccountMeta(pubkey=market_pubkey, is_signer=False, is_writable=True),
            AccountMeta(pubkey=yield_escrow, is_signer=False, is_writable=True),
            AccountMeta(pubkey=Pubkey.from_string("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"), is_signer=False, is_writable=False)
        ]

        ix = Instruction(PROGRAM_ID, instruction_data, accounts)
        blockhash_resp = await self.client.get_latest_blockhash()
        recent_blockhash = blockhash_resp.value.blockhash

        msg = MessageV0.try_compile(
            payer=self.wallet.pubkey(),
            instructions=[ix],
            address_lookup_table_accounts=[],
            recent_blockhash=recent_blockhash
        )

        tx = VersionedTransaction(msg, [self.wallet])
        res = await self.client.send_transaction(tx)
        logging.info(f"Crank submitted. Tx signature: {res.value}")

    async def run(self):
        logging.info("Starting Keeper Event Daemon Loop...")
        simulated_yield_index = 1_050_000_000_000 # 1.05e12 (+5% yield accrual)
        while True:
            try:
                # Query markets and crank execution
                markets = await self.get_market_accounts()
                logging.info(f"Active markets monitored: {len(markets)}")
                await asyncio.sleep(POLL_INTERVAL_SECONDS)
            except Exception as e:
                logging.error(f"Keeper runtime loop error: {str(e)}")
                await asyncio.sleep(POLL_INTERVAL_SECONDS)

if __name__ == "__main__":
    keeper = DominionKeeper("./id.json")
    asyncio.run(keeper.run())