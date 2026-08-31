#!/usr/bin/env node

/**
 * Health Dashboard Telemetry Polling Script
 *
 * Polls the MentorsMind health dashboard contract every 30 seconds
 * and outputs JSON metrics to a monitoring endpoint or stdout.
 *
 * Usage:
 *   node scripts/health_poller.js [--interval 30] [--rpc-url URL] [--contract-address ADDR]
 *
 * Environment variables:
 *   STELLAR_RPC_URL       - Stellar RPC URL (default: https://soroban-testnet.stellar.org)
 *   HEALTH_DASHBOARD_ID   - Contract address of the health dashboard
 *   POLL_INTERVAL_MS      - Polling interval in milliseconds (default: 30000)
 *   OUTPUT_FILE           - Optional file path to write JSON output
 */

const fs = require('fs');
const path = require('path');

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const CONFIG = {
  rpcUrl: process.env.STELLAR_RPC_URL || 'https://soroban-testnet.stellar.org',
  contractAddress: process.env.HEALTH_DASHBOARD_ID || '',
  pollIntervalMs: parseInt(process.env.POLL_INTERVAL_MS || '30000', 10),
  outputFile: process.env.OUTPUT_FILE || '',
  networkPassphrase: process.env.STELLAR_NETWORK_PASSPHRASE || 'Test SDF Network ; September 2015',
};

// Parse CLI arguments
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--interval' && args[i + 1]) {
    CONFIG.pollIntervalMs = parseInt(args[i + 1], 10);
    i++;
  } else if (args[i] === '--rpc-url' && args[i + 1]) {
    CONFIG.rpcUrl = args[i + 1];
    i++;
  } else if (args[i] === '--contract-address' && args[i + 1]) {
    CONFIG.contractAddress = args[i + 1];
    i++;
  } else if (args[i] === '--output' && args[i + 1]) {
    CONFIG.outputFile = args[i + 1];
    i++;
  }
}

// ---------------------------------------------------------------------------
// Soroban RPC interaction (simplified — use @stellar/stellar-sdk in production)
// ---------------------------------------------------------------------------

/**
 * Invoke a Soroban contract function (read-only).
 * In production, use the Stellar SDK's SorobanRpc contract.
 */
async function invokeContract(functionName, args = []) {
  const payload = {
    jsonrpc: '2.0',
    id: 1,
    method: 'simulateTransaction',
    params: {
      transaction: {
        sourceAccount: 'GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF',
        fee: '100',
        memo: { type: 'none' },
        operations: [{
          type: 'invokeHostFunction',
          body: {
            hostFunction: {
              type: 'call',
              contractAddress: CONFIG.contractAddress,
              functionName,
              args: args.map(encodeArg),
            },
            auth: [],
          },
        }],
        sequenceNumber: '0',
        timeBounds: { minTime: '0', maxTime: '0' },
        networkPassphrase: CONFIG.networkPassphrase,
      },
    },
  };

  try {
    const response = await fetch(CONFIG.rpcUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    const data = await response.json();
    if (data.error) {
      throw new Error(`RPC error: ${data.error.message}`);
    }
    return decodeResult(data.result?.result?.retval);
  } catch (err) {
    console.error(`Failed to invoke ${functionName}:`, err.message);
    return null;
  }
}

function encodeArg(value) {
  if (typeof value === 'number') {
    return { type: 'i128', value: value.toString() };
  }
  if (typeof value === 'string' && value.length === 64) {
    return { type: 'bytesN', value };
  }
  return { type: 'symbol', value };
}

function decodeResult(retval) {
  if (!retval) return null;
  // Simplified decoding — in production use Stellar SDK's xdr decoder
  return retval;
}

// ---------------------------------------------------------------------------
// Health data collection
// ---------------------------------------------------------------------------

async function collectHealthData() {
  const timestamp = new Date().toISOString();
  const ledgerTimestamp = Math.floor(Date.now() / 1000);

  const healthData = {
    timestamp,
    ledgerTimestamp,
    poller: {
      rpcUrl: CONFIG.rpcUrl,
      contractAddress: CONFIG.contractAddress,
      pollIntervalMs: CONFIG.pollIntervalMs,
    },
    metrics: {
      systemHealth: null,
      platformStats: null,
      solvencyReport: null,
      disputeStats: null,
      metricPages: [],
    },
    errors: [],
  };

  // 1. Get system health
  try {
    const healthResult = await invokeContract('is_system_healthy');
    healthData.metrics.systemHealth = healthResult;
  } catch (err) {
    healthData.errors.push({ source: 'is_system_healthy', error: err.message });
  }

  // 2. Get platform stats
  try {
    const statsResult = await invokeContract('get_platform_stats');
    healthData.metrics.platformStats = statsResult;
  } catch (err) {
    healthData.errors.push({ source: 'get_platform_stats', error: err.message });
  }

  // 3. Get solvency report
  try {
    const solvencyResult = await invokeContract('get_protocol_solvency');
    healthData.metrics.solvencyReport = solvencyResult;
  } catch (err) {
    healthData.errors.push({ source: 'get_protocol_solvency', error: err.message });
  }

  // 4. Get dispute stats
  try {
    const disputeResult = await invokeContract('get_dispute_stats');
    healthData.metrics.disputeStats = disputeResult;
  } catch (err) {
    healthData.errors.push({ source: 'get_dispute_stats', error: err.message });
  }

  // 5. Get metric pages (first 5 pages)
  try {
    const pageCountResult = await invokeContract('get_metric_page_count');
    const pageCount = typeof pageCountResult === 'number' ? pageCountResult : 0;
    const pagesToFetch = Math.min(pageCount, 5);

    for (let i = 0; i < pagesToFetch; i++) {
      try {
        const pageResult = await invokeContract('get_system_health', [i]);
        healthData.metrics.metricPages.push({
          page: i,
          metrics: pageResult,
        });
      } catch (err) {
        healthData.errors.push({ source: `get_system_health(page=${i})`, error: err.message });
      }
    }
  } catch (err) {
    healthData.errors.push({ source: 'get_metric_page_count', error: err.message });
  }

  return healthData;
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

function formatOutput(data) {
  return JSON.stringify(data, null, 2);
}

function writeOutput(data) {
  const output = formatOutput(data);

  if (CONFIG.outputFile) {
    const dir = path.dirname(CONFIG.outputFile);
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true });
    }
    fs.writeFileSync(CONFIG.outputFile, output, 'utf8');
    console.log(`[${new Date().toISOString()}] Health data written to ${CONFIG.outputFile}`);
  } else {
    console.log(output);
  }
}

// ---------------------------------------------------------------------------
// Main polling loop
// ---------------------------------------------------------------------------

let pollCount = 0;

async function poll() {
  pollCount++;
  console.error(`[${new Date().toISOString()}] Poll #${pollCount}...`);

  try {
    const healthData = await collectHealthData();
    healthData.pollCount = pollCount;
    writeOutput(healthData);

    // Log summary to stderr
    const { systemHealth, platformStats, solvencyReport } = healthData.metrics;
    if (systemHealth) {
      console.error(`  System healthy: ${systemHealth.is_healthy}`);
      console.error(`  Total metrics: ${systemHealth.total_metrics}`);
      console.error(`  Critical alerts: ${systemHealth.critical_count}`);
    }
    if (platformStats) {
      console.error(`  TVL: ${platformStats.total_value_locked}`);
      console.error(`  Active escrows: ${platformStats.active_escrows}`);
    }
    if (solvencyReport) {
      console.error(`  Solvent: ${solvencyReport.is_solvent}`);
    }
  } catch (err) {
    console.error(`[${new Date().toISOString()}] Poll error:`, err.message);
  }
}

// Start polling
console.error(`Health Dashboard Poller starting...`);
console.error(`  RPC URL: ${CONFIG.rpcUrl}`);
console.error(`  Contract: ${CONFIG.contractAddress || '(not set)'}`);
console.error(`  Interval: ${CONFIG.pollIntervalMs}ms`);
console.error(`  Output: ${CONFIG.outputFile || '(stdout)'}`);
console.error('');

if (!CONFIG.contractAddress) {
  console.error('WARNING: HEALTH_DASHBOARD_ID not set. Polling will fail.');
  console.error('Set it via environment variable or --contract-address flag.');
}

// Run first poll immediately, then schedule interval
poll();
setInterval(poll, CONFIG.pollIntervalMs);

// Handle graceful shutdown
process.on('SIGINT', () => {
  console.error('\nShutting down poller...');
  process.exit(0);
});

process.on('SIGTERM', () => {
  console.error('\nShutting down poller...');
  process.exit(0);
});
