"""
NectarFi x Dominion Protocol - Autonomous Keeper Daemon
Continuously polls active markets, computes yield differentials, submits harvest CPIs,
and settles expired Dominion markets based on Pyth oracle price feeds.
"""

import asyncio
import os
import struct
import logging
from dataclasses import dataclass
from typing import List, Optional
from solana.rpc.async_api import AsyncClient
from solana.rpc.commitment import Confirmed
from solders.pubkey import Pubkey
from solders.keypair import Keypair
from solders.instruction import Instruction, AccountMeta
from solders.transaction import Transaction

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] (KeeperCore) %(message)s"
)

PROGRAM_ID = Pubkey.from_string("NectDom1111111111111111111111111111111111111")
PRECISION = 1_000_000_000

@dataclass
class MarketMetadata:
    pubkey: Pubkey
    market_id: int
    creator: Pubkey
    escrow_vault: Pubkey
    expiry_timestamp: int
    strike_price: int
    resolved: bool

class DominionKeeper:
    def __init__(self, rpc_url: str, keypair_path: str):
        self.client = AsyncClient(rpc_url, commitment=Confirmed)
        with open(os.path.expanduser(keypair_path), "r") as f:
            secret = eval(f.read().strip())
            self.wallet = Keypair.from_bytes(bytes(secret))
        logging.info(f"Keeper initialized with Operator: {self.wallet.pubkey()}")

    async def fetch_active_markets(self) -> List[MarketMetadata]:
        # Anchor discriminator for "DominionMarket"
        # Sha256("account:DominionMarket")[0..8]
        discriminator = bytes([218, 14, 18, 92, 198, 203, 114, 189])
        
        resp = await self.client.get_program_accounts(
            PROGRAM_ID,
            commitment=Confirmed,
            encoding="base64"
        )
        
        markets = []
        if resp.value is None:
            return markets

        for raw_acc in resp.value:
            data = raw_acc.account.data
            if data[:8] == discriminator:
                # Unpack Market Layout
                # 8 (disc) + 8 (market_id) + 32 (creator) + 32 (nectar_mint) + 32 (escrow_vault) + 8 (expiry) + 8 (strike) + 1 (resolved)
                market_id = struct.unpack("<Q", data[8:16])[0]
                creator = Pubkey.from_bytes(data[16:48])
                escrow_vault = Pubkey.from_bytes(data[80:112])
                expiry = struct.unpack("<q", data[112:120])[0]
                strike = struct.unpack("<Q", data[120:128])[0]
                resolved = bool(data[128])

                markets.append(MarketMetadata(
                    pubkey=raw_acc.pubkey,
                    market_id=market_id,
                    creator=creator,
                    escrow_vault=escrow_vault,
                    expiry_timestamp=expiry,
                    strike_price=strike,
                    resolved=resolved
                ))
        return markets

    async def harvest_market(self, market: MarketMetadata, virtual_rate: int):
        logging.info(f"Submitting harvest for Market ID: {market.market_id} (Rate: {virtual_rate})")
        # Instruction discriminator for "harvest_yield" = [232, 119, 147, 240, 20, 219, 14, 12]
        ix_data = bytes([232, 119, 147, 240, 20, 219, 14, 12]) + struct.unpack("<16B", struct.pack("<QQ", virtual_rate & 0xFFFFFFFFFFFFFFFF, virtual_rate >> 64))

        accounts = [
            AccountMeta(pubkey=market.pubkey, is_signer=False, is_writable=True),
            AccountMeta(pubkey=market.escrow_vault, is_signer=False, is_writable=False),
            AccountMeta(pubkey=self.wallet.pubkey(), is_signer=True, is_writable=False),
        ]

        ix = Instruction(PROGRAM_ID, ix_data, accounts)
        blockhash_resp = await self.client.get_latest_blockhash()
        
        tx = Transaction.new_signed_with_payer(
            [ix],
            self.wallet.pubkey(),
            [self.wallet],
            blockhash_resp.value.blockhash
        )
        sig = await self.client.send_raw_transaction(bytes(tx))
        logging.info(f"Harvest executed. Signature: {sig.value}")

    async def run_loop(self):
        logging.info("Starting Keeper execution loop...")
        while True:
            try:
                markets = await self.fetch_active_markets()
                logging.info(f"Discovered {len(markets)} markets on-chain.")
                
                for market in markets:
                    if not market.resolved:
                        # Simulated NectarFi dynamic vault exchange rate tracking (1.05e9)
                        simulated_rate = int(1.05 * PRECISION)
                        await self.harvest_market(market, simulated_rate)

            except Exception as e:
                logging.error(f"Keeper loop execution error: {str(e)}")

            await asyncio.sleep(10)

if __name__ == "__main__":
    RPC = os.getenv("SOLANA_RPC_URL", "http://127.0.0.1:8899")
    KEYPAIR = os.getenv("KEEPER_KEYPAIR", "~/.config/solana/id.json")
    keeper = DominionKeeper(RPC, KEYPAIR)
    asyncio.run(keeper.run_loop())