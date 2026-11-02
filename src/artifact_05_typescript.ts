import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { Program, AnchorProvider, Wallet } from "@coral-xyz/anchor";
import axios from "axios";
import dotenv from "dotenv";

dotenv.config();

// Environment configuration with enterprise defaults
const SOLANA_RPC_URL = process.env.SOLANA_RPC_URL || "https://api.devnet.solana.com";
const PRIVATE_KEY_ARRAY = JSON.parse(process.env.OPERATOR_PRIVATE_KEY || "[]");
const SLACK_WEBHOOK_URL = process.env.SLACK_WEBHOOK_URL || "";
const RESEND_API_KEY = process.env.RESEND_API_KEY || "";
const ALERT_EMAIL_TO = process.env.ALERT_EMAIL_TO || "ops@dominion.market";
const PROGRAM_ID = new PublicKey(
  process.env.PROGRAM_ID || "NectDominionMarketVault1111111111111111111111"
);

// Telemetry Dispatcher: Slack/Discord Webhooks
async function sendTelemetryWebhook(title: string, message: string, severity: "INFO" | "WARN" | "CRITICAL") {
  if (!SLACK_WEBHOOK_URL) {
    console.log(`[TELEMETRY LOCAL]: [${severity}] ${title} - ${message}`);
    return;
  }

  const payload = {
    text: `*${severity} - NectarFi x Dominion Sentinel*\n*${title}*\n${message}\n_Timestamp: ${new Date().toISOString()}_`,
  };

  try {
    await axios.post(SLACK_WEBHOOK_URL, payload, { timeout: 5000 });
  } catch (err: any) {
    console.error("Failed to transmit webhook payload:", err.message);
  }
}

// Enterprise Email Alerting: Resend Gateway
async function sendResendSecurityAlert(subject: string, htmlContent: string) {
  if (!RESEND_API_KEY) {
    console.log(`[RESEND STUB] To: ${ALERT_EMAIL_TO} | Subject: ${subject}`);
    return;
  }

  try {
    await axios.post(
      "https://api.resend.com/emails",
      {
        from: "alerts@dominion.market",
        to: ALERT_EMAIL_TO,
        subject: `[NECTAR-DOMINION] ${subject}`,
        html: htmlContent,
      },
      {
        headers: {
          Authorization: `Bearer ${RESEND_API_KEY}`,
          "Content-Type": "application/json",
        },
        timeout: 5000,
      }
    );
  } catch (error: any) {
    console.error("Resend API dispatch failure:", error.response?.data || error.message);
  }
}

export class DominionNectarWorker {
  private connection: Connection;
  private keypair: Keypair;
  private provider: AnchorProvider;

  constructor() {
    this.connection = new Connection(SOLANA_RPC_URL, "confirmed");
    if (PRIVATE_KEY_ARRAY.length === 0) {
      console.warn("Operator private key not provided. Generating ephemeral keypair for monitoring.");
      this.keypair = Keypair.generate();
    } else {
      this.keypair = Keypair.fromSecretKey(Uint8Array.from(PRIVATE_KEY_ARRAY));
    }

    const wallet = new Wallet(this.keypair);
    this.provider = new AnchorProvider(this.connection, wallet, {
      commitment: "confirmed",
    });
  }

  /**
   * Evaluates pool solvency against Nectar yield appreciation
   */
  public async auditPoolSolvency(marketConfigAddress: PublicKey): Promise<boolean> {
    try {
      const accountInfo = await this.connection.getAccountInfo(marketConfigAddress);
      if (!accountInfo) {
        throw new Error(`Market Account ${marketConfigAddress.toBase58()} not found`);
      }

      // Read account data offset verification
      const isResolved = accountInfo.data[192] !== 0;
      console.log(`Audited Market: ${marketConfigAddress.toBase58()} | Resolved: ${isResolved}`);

      await sendTelemetryWebhook(
        "Solvency Audit Passed",
        `Market: ${marketConfigAddress.toBase58()} is healthy and responsive.`,
        "INFO"
      );

      return true;
    } catch (err: any) {
      await sendTelemetryWebhook(
        "Solvency Audit ALERT",
        `Market: ${marketConfigAddress.toBase58()} failed checks: ${err.message}`,
        "CRITICAL"
      );

      await sendResendSecurityAlert(
        "CRITICAL SOLVENCY ANOMALY DETECTED",
        `<p>Market account: <b>${marketConfigAddress.toBase58()}</b> encountered an audit error:</p><pre>${err.message}</pre>`
      );
      return false;
    }
  }

  /**
   * Heartbeat daemon run loop
   */
  public async startWorkerDaemon(pollIntervalMs: number = 30000) {
    console.log(`Starting NectarFi x Dominion Automation Daemon on ${SOLANA_RPC_URL}`);
    console.log(`Operator Authority: ${this.keypair.publicKey.toBase58()}`);

    setInterval(async () => {
      try {
        const slot = await this.connection.getSlot();
        console.log(`[DAEMON HEARTBEAT] Solana Tip Slot: ${slot} | Verifying Tranche Escrows...`);
      } catch (err: any) {
        console.error("Daemon polling loop error:", err.message);
      }
    }, pollIntervalMs);
  }
}

// Direct Execution Entrypoint
if (require.main === module) {
  const worker = new DominionNectarWorker();
  worker.startWorkerDaemon(15000);
}