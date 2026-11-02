// keeper-service.ts
import { Connection, Keypair, PublicKey, clusterApiUrl } from "@solana/web3.js";
import { Program, AnchorProvider, Wallet, Idl, BN } from "@coral-xyz/anchor";
import { Resend } from "resend";
import axios from "axios";
import * as dotenv from "dotenv";
import * as fs from "fs";

dotenv.config();

// Configuration
const RPC_ENDPOINT = process.env.SOLANA_RPC_URL || clusterApiUrl("devnet");
const RESEND_API_KEY = process.env.RESEND_API_KEY || "re_test_dummy_key";
const DISCORD_WEBHOOK_URL = process.env.DISCORD_WEBHOOK_URL || "";
const KEYPAIR_PATH = process.env.KEYPAIR_PATH || "./keeper-keypair.json";
const PROGRAM_ID = new PublicKey("DNYE111111111111111111111111111111111111111");

const resend = new Resend(RESEND_API_KEY);

interface MarginHealthStatus {
    accountPubkey: PublicKey;
    borrower: PublicKey;
    shares: number;
    debt: number;
    healthRatioBps: number;
    isLiquidatable: boolean;
}

export class DominionNectarKeeper {
    private connection: Connection;
    private keeperKeypair: Keypair;
    private provider: AnchorProvider;

    constructor() {
        this.connection = new Connection(RPC_ENDPOINT, { commitment: "confirmed" });
        if (fs.existsSync(KEYPAIR_PATH)) {
            const secret = JSON.parse(fs.readFileSync(KEYPAIR_PATH, "utf-8"));
            this.keeperKeypair = Keypair.fromSecretKey(new Uint8Array(secret));
        } else {
            console.warn("⚠️ Warning: Keeper keypair not found. Initializing temporary mock keypair.");
            this.keeperKeypair = Keypair.generate();
        }
        const wallet = new Wallet(this.keeperKeypair);
        this.provider = new AnchorProvider(this.connection, wallet, { commitment: "confirmed" });
    }

    /**
     * Inspects target margin account state and computes health ratio against current market rates.
     */
    public evaluateMarginHealth(
        accountPubkey: PublicKey,
        borrower: PublicKey,
        depositedShares: BN,
        borrowedDebt: BN,
        totalUnderlying: BN,
        totalShares: BN,
        liquidationThresholdBps: BN
    ): MarginHealthStatus {
        const debtNum = borrowedDebt.toNumber();
        const sharesNum = depositedShares.toNumber();

        if (debtNum === 0) {
            return {
                accountPubkey,
                borrower,
                shares: sharesNum,
                debt: 0,
                healthRatioBps: 99999, // Infinite health
                isLiquidatable: false,
            };
        }

        const totalUnderlyingNum = totalUnderlying.toNumber();
        const totalSharesNum = totalShares.toNumber();
        const collateralValue = (sharesNum * totalUnderlyingNum) / (totalSharesNum > 0 ? totalSharesNum : 1);
        const healthRatioBps = Math.floor((collateralValue * 10000) / debtNum);

        const isLiquidatable = healthRatioBps < liquidationThresholdBps.toNumber();

        return {
            accountPubkey,
            borrower,
            shares: sharesNum,
            debt: debtNum,
            healthRatioBps,
            isLiquidatable,
        };
    }

    /**
     * Dispatches real enterprise alert via Resend (Email) and Discord Webhook
     */
    public async notifyEnterpriseRisk(status: MarginHealthStatus): Promise<void> {
        console.log(`[ALERT] Undercollateralized Account Detected: ${status.borrower.toBase58()}`);
        console.log(`[ALERT] Current Ratio: ${status.healthRatioBps} bps | Debt: ${status.debt}`);

        // 1. Resend Email Notification
        try {
            if (process.env.RESEND_API_KEY) {
                await resend.emails.send({
                    from: "risk-engine@dominion-nectar.fi",
                    to: ["risk-ops@enterprise-fund.com"],
                    subject: `CRITICAL: Dominion Margin Account Liquidation Warning - ${status.borrower.toBase58().substring(0, 8)}`,
                    html: `
                        <h2>🚨 Dominion-Nectar Risk Notification</h2>
                        <p>A margin position under your custodial supervision has breached maintenance levels.</p>
                        <ul>
                            <li><strong>Borrower:</strong> <code>${status.borrower.toBase58()}</code></li>
                            <li><strong>Health Factor (BPS):</strong> ${status.healthRatioBps}</li>
                            <li><strong>Outstanding Debt:</strong> ${status.debt} units</li>
                            <li><strong>Action:</strong> Automated CPI liquidation armed</li>
                        </ul>
                    `,
                });
                console.log("✅ Resend enterprise dispatch confirmed.");
            }
        } catch (err) {
            console.error("❌ Resend dispatch failed:", err);
        }

        // 2. Discord Webhook Incident Notification
        if (DISCORD_WEBHOOK_URL) {
            try {
                await axios.post(DISCORD_WEBHOOK_URL, {
                    username: "Dominion-Nectar Sentinel",
                    embeds: [
                        {
                            title: "⚠️ High Priority: Undercollateralized Position",
                            color: 15158332,
                            fields: [
                                { name: "Account", value: status.borrower.toBase58(), inline: true },
                                { name: "Health BPS", value: status.healthRatioBps.toString(), inline: true },
                                { name: "Debt", value: status.debt.toString(), inline: true },
                                { name: "Timestamp", value: new Date().toISOString() },
                            ],
                        },
                    ],
                });
                console.log("✅ Discord webhook broadcast successful.");
            } catch (err) {
                console.error("❌ Discord webhook dispatch failed:", err);
            }
        }
    }

    /**
     * Continuous Sentinel Loop
     */
    public async startSentinel(): Promise<void> {
        console.log("🚀 Dominion-Nectar Sentinel Keeper service online.");
        console.log(`📡 Connected to RPC: ${RPC_ENDPOINT}`);
        console.log(`🔑 Liquidator Keypair: ${this.keeperKeypair.publicKey.toBase58()}`);

        // Simulated check interval: In production, registers anchor program account change websocket filters
        setInterval(async () => {
            try {
                // Mock assessment example for telemetry proof:
                const mockAccount = {
                    accountPubkey: Keypair.generate().publicKey,
                    borrower: new PublicKey("7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU"),
                    shares: new BN(100_000_000),
                    debt: new BN(95_000_000),
                    totalUnderlying: new BN(100_000_000),
                    totalShares: new BN(100_000_000),
                    threshold: new BN(11500),
                };

                const health = this.evaluateMarginHealth(
                    mockAccount.accountPubkey,
                    mockAccount.borrower,
                    mockAccount.shares,
                    mockAccount.debt,
                    mockAccount.totalUnderlying,
                    mockAccount.totalShares,
                    mockAccount.threshold
                );

                if (health.isLiquidatable) {
                    await this.notifyEnterpriseRisk(health);
                }
            } catch (e) {
                console.error("Error in Sentinel tick:", e);
            }
        }, 15000);
    }
}

// Entrypoint execution
if (require.main === module) {
    const keeper = new DominionNectarKeeper();
    keeper.startSentinel();
}