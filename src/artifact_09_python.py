# keeper/crank_bot.py
import time
import asyncio
from solana.rpc.async_api import AsyncClient
from anchorpy import Provider, Wallet, Program
from solders.keypair import Keypair

async def monitor_and_roll():
    client = AsyncClient("https://api.devnet.solana.com")
    keeper_wallet = Wallet(Keypair.from_base58_string("<KEEPER_PRIVATE_KEY>"))
    provider = Provider(client, keeper_wallet)
    program = await Program.at("NectAr1111111111111111111111111111111111111", provider)

    print("[*] NectarFi Dominion Keeper Bot Active...")
    while True:
        try:
            vault_data = await program.account["VaultConfig"].fetch("<VAULT_CONFIG_PDA>")
            current_time = int(time.time())
            
            if current_time >= vault_data.tranche_maturity:
                print(f"[!] Tranche matured at {vault_data.tranche_maturity}. Executing rollover...")
                next_maturity = current_time + (30 * 86400)
                tx = await program.methods.execute_tranche_rollover(next_maturity, 850)\
                    .accounts({...})\
                    .rpc()
                print(f"[+] Rollover Success! Tx Sig: {tx}")
            else:
                print(f"[-] Tranche active. Time remaining: {vault_data.tranche_maturity - current_time}s")
        except Exception as e:
            print(f"[ERR] Keeper cycle failure: {e}")
            
        await asyncio.sleep(15)

if __name__ == "__main__":
    asyncio.run(monitor_and_roll())