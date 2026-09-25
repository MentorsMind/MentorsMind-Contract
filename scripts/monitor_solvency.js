#!/usr/bin/env node

/**
 * Solvency Monitoring Script (Issue #771)
 *
 * Polls the HealthDashboard contract's `get_protocol_solvency` view
 * every 5 minutes and pages on insolvency detection.
 *
 * Usage:
 *   node monitor_solvency.js [--rpc-url <url>] [--dashboard-id <id>]
 *
 * Environment variables:
 *   RPC_URL          — Stellar RPC endpoint (default: http://localhost:8003)
 *   DASHBOARD_ID     — HealthDashboard contract ID (default: from env)
 *   PAGER_TOKEN      — Optional pager integration token (e.g. PagerDuty)
 *   PAGER_ENDPOINT   — Optional pager webhook URL
 *
 * Exit codes:
 *   0 — healthy (all checks passed)
 *   1 — insolvent detected
 *   2 — configuration error / RPC failure
 */

const STELLAR_RPC_URL = process.env.RPC_URL || 'http://localhost:8003';
const DASHBOARD_ID = process.env.DASHBOARD_ID || '';
const PAGER_TOKEN = process.env.PAGER_TOKEN || '';
const PAGER_ENDPOINT = process.env.PAGER_ENDPOINT || '';
const POLL_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes
const MAX_RETRIES = 3;
const RETRY_DELAY_MS = 5_000;

// ---------------------------------------------------------------------------
// Simple Stellar RPC caller (Soroban simulateTransaction)
// ---------------------------------------------------------------------------

async function simulateContractCall(contractId, functionName, args = []) {
    const body = {
        jsonrpc: '2.0',
        id: 1,
        method: 'simulateTransaction',
        params: {
            transaction: {
                sourceAccount: 'GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF',
                operations: [{
                    body: {
                        type: 'invokeHostFunction',
                        invokeHostFunctionOp: {
                            function: 'hostFunction',
                            hostFunction: {
                                type: 'invokeContract',
                                contractId: contractId,
                                functionName: functionName,
                                args: args,
                            },
                        },
                    },
                }],
            },
        },
    };

    for (let attempt = 0; attempt < MAX_RETRIES; attempt++) {
        try {
            const response = await fetch(STELLAR_RPC_URL, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(body),
            });

            if (!response.ok) {
                throw new Error(`RPC HTTP ${response.status}: ${response.statusText}`);
            }

            const data = await response.json();
            if (data.error) {
                throw new Error(`RPC error: ${JSON.stringify(data.error)}`);
            }

            // Parse result from simulateTransaction response
            const result = data.result;
            if (!result) {
                throw new Error('No result in simulateTransaction response');
            }

            return result;
        } catch (err) {
            if (attempt < MAX_RETRIES - 1) {
                console.warn(`[WARN] RPC call failed (attempt ${attempt + 1}/${MAX_RETRIES}): ${err.message}`);
                await sleep(RETRY_DELAY_MS);
            } else {
                throw err;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Solvency report decoder (parses ScVal from Soroban response)
// ---------------------------------------------------------------------------

function decodeScVal(val) {
    if (!val) return null;

    switch (val.type) {
        case 'scvBool':
            return val.value;
        case 'scvI128':
            const parts = val.value;
            if (Array.isArray(parts)) {
                // Parts: [lo, hi] in hex
                const lo = BigInt(parts[0] || '0x0');
                const hi = BigInt(parts[1] || '0x0');
                return hi < 0 ? -(((~hi & BigInt('0xFFFFFFFFFFFFFFFF')) << BigInt(64)) | (~lo & BigInt('0xFFFFFFFFFFFFFFFF')) + BigInt(1)) : (hi << BigInt(64)) | lo;
            }
            return BigInt(parts);
        case 'scvMap':
            const map = {};
            for (const entry of val.value) {
                const key = decodeScVal(entry.key);
                const value = decodeScVal(entry.val);
                map[key] = value;
            }
            return map;
        case 'scvSymbol':
            return val.value;
        case 'scvU32':
            return val.value;
        case 'scvU64':
            return typeof val.value === 'object' ? BigInt(val.value.lo) | (BigInt(val.value.hi) << BigInt(32)) : BigInt(val.value);
        case 'scvAddress':
            return val.value;
        default:
            return val;
    }
}

function parseSolvencyReport(scVal) {
    const map = decodeScVal(scVal);
    if (!map || typeof map !== 'object') {
        throw new Error('Failed to decode SolvencyReport from ScVal');
    }

    return {
        treasury_balance: BigInt(map['treasury_balance'] || 0),
        pending_allocations: BigInt(map['pending_allocations'] || 0),
        insurance_pool_balance: BigInt(map['insurance_pool_balance'] || 0),
        outstanding_claims: BigInt(map['outstanding_claims'] || 0),
        staking_total: BigInt(map['staking_total'] || 0),
        pending_rewards: BigInt(map['pending_rewards'] || 0),
        lending_total_liquidity: BigInt(map['lending_total_liquidity'] || 0),
        outstanding_loans: BigInt(map['outstanding_loans'] || 0),
        is_solvent: Boolean(map['is_solvent']),
    };
}

// ---------------------------------------------------------------------------
// Pager integration
// ---------------------------------------------------------------------------

async function pageOnCall(message) {
    console.error(`[ALERT] Paging: ${message}`);

    if (PAGER_ENDPOINT) {
        const headers = { 'Content-Type': 'application/json' };
        if (PAGER_TOKEN) {
            headers['Authorization'] = `Bearer ${PAGER_TOKEN}`;
        }

        try {
            const response = await fetch(PAGER_ENDPOINT, {
                method: 'POST',
                headers,
                body: JSON.stringify({
                    severity: 'critical',
                    source: 'mentormind-solvency-monitor',
                    message,
                    timestamp: new Date().toISOString(),
                }),
            });
            if (!response.ok) {
                console.error(`[ERROR] Failed to page: HTTP ${response.status}`);
            } else {
                console.log('[INFO] Pager notified successfully');
            }
        } catch (err) {
            console.error(`[ERROR] Pager call failed: ${err.message}`);
        }
    } else {
        console.log('[INFO] No pager endpoint configured — alert logged only');
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

function formatBalance(balance) {
    // Format as human-readable USDC (assumes 7-decimal places for Stellar)
    const divisor = BigInt(10_000_000);
    const whole = balance / divisor;
    const fraction = balance % divisor;
    const fractionStr = fraction.toString().padStart(7, '0').slice(0, 2);
    return `${whole}.${fractionStr}`;
}

function formatBigInt(n) {
    return n.toString();
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

async function checkSolvency() {
    if (!DASHBOARD_ID) {
        console.error('[FATAL] DASHBOARD_ID not set. Set DASHBOARD_ID env var or pass --dashboard-id');
        process.exit(2);
    }

    console.log(`\n========================================`);
    console.log(`Solvency Check at ${new Date().toISOString()}`);
    console.log(`Dashboard: ${DASHBOARD_ID}`);
    console.log(`RPC: ${STELLAR_RPC_URL}`);
    console.log(`========================================`);

    try {
        const result = await simulateContractCall(DASHBOARD_ID, 'get_protocol_solvency', []);
        if (!result || !result.result) {
            throw new Error('Empty result from simulateTransaction');
        }

        const report = parseSolvencyReport(result.result);

        console.log(`  Treasury Balance:       ${formatBalance(report.treasury_balance)}`);
        console.log(`  Pending Allocations:    ${formatBalance(report.pending_allocations)}`);
        console.log(`  Insurance Pool:         ${formatBalance(report.insurance_pool_balance)}`);
        console.log(`  Outstanding Claims:     ${formatBalance(report.outstanding_claims)}`);
        console.log(`  Staking Total:          ${formatBigInt(report.staking_total)}`);
        console.log(`  Pending Rewards:        ${formatBigInt(report.pending_rewards)}`);
        console.log(`  Lending Liquidity:      ${formatBalance(report.lending_total_liquidity)}`);
        console.log(`  Outstanding Loans:      ${formatBigInt(report.outstanding_loans)}`);
        console.log(`  Solvent:                ${report.is_solvent ? 'YES' : 'NO'}`);

        if (!report.is_solvent) {
            const msg = [
                `PROTOCOL INSOLVENT DETECTED`,
                `Treasury: ${formatBalance(report.treasury_balance)} (pending: ${formatBalance(report.pending_allocations)})`,
                `Insurance: ${formatBalance(report.insurance_pool_balance)} (claims: ${formatBalance(report.outstanding_claims)})`,
                `Lending: ${formatBalance(report.lending_total_liquidity)} (loans: ${formatBigInt(report.outstanding_loans)})`,
                `Staking: ${formatBigInt(report.staking_total)} (rewards: ${formatBigInt(report.pending_rewards)})`,
            ].join(' | ');

            console.error(`[CRITICAL] ${msg}`);
            await pageOnCall(msg);
            return false;
        }

        console.log(`[OK] Protocol is solvent`);
        return true;
    } catch (err) {
        console.error(`[ERROR] Solvency check failed: ${err.message}`);
        console.error(err.stack);
        return null; // indeterminate
    }
}

async function main() {
    const args = process.argv.slice(2);
    let continuousMode = true;

    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--rpc-url' && args[i + 1]) {
            process.env.RPC_URL = args[i + 1];
            i++;
        } else if (args[i] === '--dashboard-id' && args[i + 1]) {
            process.env.DASHBOARD_ID = args[i + 1];
            i++;
        } else if (args[i] === '--once') {
            continuousMode = false;
        }
    }

    if (!process.env.DASHBOARD_ID) {
        console.error('Error: DASHBOARD_ID environment variable or --dashboard-id argument is required');
        console.error('');
        console.error('Usage: node monitor_solvency.js [--rpc-url <url>] [--dashboard-id <id>] [--once]');
        process.exit(2);
    }

    console.log('=== MentorsMind Protocol Solvency Monitor ===');
    console.log(`Poll interval: ${POLL_INTERVAL_MS / 1000}s`);
    console.log(`Continuous mode: ${continuousMode}`);
    console.log('');

    let lastAlertAt = 0;
    const ALERT_COOLDOWN_MS = 30 * 60 * 1000; // re-alert at most once per 30 min

    do {
        const isSolvent = await checkSolvency();

        if (isSolvent === false) {
            const now = Date.now();
            if (now - lastAlertAt > ALERT_COOLDOWN_MS) {
                console.error('[ALERT] Insolvency detected — triggering alert');
                lastAlertAt = now;
            }

            if (!continuousMode) {
                console.error('[EXIT] Insolvent — exiting with code 1');
                process.exit(1);
            }
        }

        if (continuousMode) {
            console.log(`[INFO] Next check in ${POLL_INTERVAL_MS / 1000}s...`);
            await sleep(POLL_INTERVAL_MS);
        }
    } while (continuousMode);

    // If --once mode and we got here, solvent
    console.log('[OK] Solvent — exiting with code 0');
    process.exit(0);
}

main().catch(err => {
    console.error(`[FATAL] ${err.message}`);
    process.exit(2);
});

