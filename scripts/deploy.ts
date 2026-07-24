#!/usr/bin/env node
// deploy.ts — TypeScript alternative using @stellar/stellar-sdk
// Run: npx ts-node scripts/deploy.ts [--network testnet|mainnet]
import { execSync } from "child_process";
import * as fs from "fs";
import * as path from "path";

// ── Config ────────────────────────────────────────────────────────────────────
const args = process.argv.slice(2);
const networkIdx = args.indexOf("--network");
const NETWORK: string = networkIdx !== -1 ? args[networkIdx + 1] : "testnet";
const IDENTITY = process.env.STELLAR_IDENTITY ?? "default";

const REPO_ROOT = path.resolve(__dirname, "..");
const CONFIG_FILE = path.join(REPO_ROOT, "deployed", `${NETWORK}.json`);

const NETWORK_CONFIG: Record<string, { rpc: string; passphrase: string; friendbot?: string }> = {
  testnet: {
    rpc: "https://soroban-testnet.stellar.org:443",
    passphrase: "Test SDF Network ; September 2015",
    friendbot: "https://friendbot.stellar.org",
  },
  mainnet: {
    rpc: "https://mainnet.stellar.validationcloud.io/v1/",
    passphrase: "Public Global Stellar Network ; September 2015",
  },
};

if (!NETWORK_CONFIG[NETWORK]) {
  console.error(`Unknown network: ${NETWORK}`); process.exit(1);
}

// ── Helpers ───────────────────────────────────────────────────────────────────
function run(cmd: string): string {
  return execSync(cmd, { encoding: "utf8" }).trim();
}

function loadConfig(): Record<string, string> {
  if (!fs.existsSync(CONFIG_FILE)) return {};
  return JSON.parse(fs.readFileSync(CONFIG_FILE, "utf8"));
}

function saveConfig(cfg: Record<string, string>): void {
  fs.mkdirSync(path.dirname(CONFIG_FILE), { recursive: true });
  fs.writeFileSync(CONFIG_FILE, JSON.stringify(cfg, null, 2));
}

const STELLAR = `stellar --network ${NETWORK} --source ${IDENTITY}`;

function deployContract(name: string, wasm: string, cfg: Record<string, string>): string {
  if (cfg[name]) {
    console.log(`  ↷ ${name} (already deployed) → ${cfg[name]}`);
    return cfg[name];
  }
  console.log(`[deploy] Deploying ${name} …`);
  const id = run(`${STELLAR} contract deploy --wasm ${wasm}`);
  cfg[name] = id;
  saveConfig(cfg);
  console.log(`  ✓ ${name} → ${id}`);
  return id;
}

function invoke(contractId: string, ...fnArgs: string[]): string {
  try {
    return run(`${STELLAR} contract invoke --id ${contractId} -- ${fnArgs.join(" ")}`);
  } catch (e: any) {
    const msg: string = e.stderr ?? e.message ?? "";
    if (msg.includes("Already initialized")) return "(already initialized)";
    throw e;
  }
}

// ── Main ──────────────────────────────────────────────────────────────────────
async function main() {
  const { rpc, passphrase, friendbot } = NETWORK_CONFIG[NETWORK];

  // Register network
  try {
    run(`stellar network add ${NETWORK} --rpc-url ${rpc} --network-passphrase "${passphrase}"`);
  } catch { /* already registered */ }

  const admin = run(`stellar keys address ${IDENTITY}`);

  // Fund via Friendbot (testnet only)
  if (friendbot) {
    console.log(`[deploy] Funding ${admin} via Friendbot …`);
    try {
      run(`curl -sf "${friendbot}?addr=${admin}" -o /dev/null`);
      console.log("  ✓ Funded");
    } catch { console.log("  ↷ Already funded"); }
  }

  // Build
  console.log("[deploy] Building contracts …");
  run(`cd ${REPO_ROOT} && cargo build --target wasm32-unknown-unknown --release -q`);
  console.log("  ✓ Build complete");

  const WASM = `${REPO_ROOT}/target/wasm32-unknown-unknown/release`;
  const cfg = loadConfig();

  // Deploy
  const escrowId       = deployContract("escrow",       `${WASM}/mentorminds_escrow.wasm`,       cfg);
  const verificationId = deployContract("verification", `${WASM}/mentorminds_verification.wasm`, cfg);
  const tokenId        = deployContract("mnt_token",    `${WASM}/mentorminds_mnt_token.wasm`,    cfg);

  // Initialize
  console.log("[deploy] Initializing contracts …");
  invoke(escrowId, "initialize",
    "--admin", admin, "--treasury", admin,
    "--fee_bps", "500", "--approved_tokens", "[]",
    "--auto_release_delay_secs", "259200");
  console.log("  ✓ escrow initialized");

  invoke(verificationId, "initialize", "--admin", admin);
  console.log("  ✓ verification initialized");

  invoke(tokenId, "initialize", "--admin", admin);
  console.log("  ✓ mnt_token initialized");

  // Verify
  console.log("[deploy] Verifying …");
  const fee = invoke(escrowId, "get_fee_bps");
  console.log(`  ✓ escrow.get_fee_bps → ${fee}`);
  const isVer = invoke(verificationId, "is_verified", "--mentor", admin);
  console.log(`  ✓ verification.is_verified → ${isVer}`);

  // Metadata
  cfg.network     = NETWORK;
  cfg.admin       = admin;
  cfg.deployed_at = new Date().toISOString();
  saveConfig(cfg);

  // Summary
  const contracts = [
    ["escrow",       escrowId],
    ["verification", verificationId],
    ["mnt_token",    tokenId],
  ];
  console.log("\n┌─────────────────────┬──────────────────────────────────────────────────────────┐");
  console.log(`│ ${"Contract".padEnd(19)} │ ${"ID".padEnd(56)} │`);
  console.log("├─────────────────────┼──────────────────────────────────────────────────────────┤");
  for (const [name, id] of contracts) {
    console.log(`│ ${name.padEnd(19)} │ ${id.padEnd(56)} │`);
  }
  console.log("└─────────────────────┴──────────────────────────────────────────────────────────┘");
  console.log(`\nConfig saved → ${CONFIG_FILE}`);
}

main().catch((e) => { console.error(e); process.exit(1); });
