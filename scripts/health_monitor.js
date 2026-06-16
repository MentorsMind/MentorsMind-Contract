const fetch = require('node-fetch');

const HEALTH_DASHBOARD_ID = process.env.HEALTH_DASHBOARD_ID || 'C_DUMMY_DASHBOARD_ID';
const RPC_URL = process.env.RPC_URL || 'https://rpc-futurenet.stellar.org:443';
const MONITORING_ENDPOINT = process.env.MONITORING_ENDPOINT || 'http://localhost:8080/metrics';
const POLLING_INTERVAL = 30000; // 30 seconds

async function pollHealthDashboard() {
    console.log(`[${new Date().toISOString()}] Polling Health Dashboard...`);
    
    try {
        // In a real-world scenario, you would use @stellar/stellar-sdk to invoke the Soroban 
        // `get_system_health` function. For example:
        // const contract = new Contract(HEALTH_DASHBOARD_ID);
        // const res = await contract.call("get_system_health", 0, 100);
        
        // Here we simulate receiving the paginated HealthMetric array
        const mockResponse = [
            { contract: "escrow", metric: "active_count", value: 120, timestamp: Date.now() },
            { contract: "staking", metric: "tvl", value: 450000, timestamp: Date.now() },
            { contract: "governance", metric: "pending_proposals", value: 3, timestamp: Date.now() },
            { contract: "oracle", metric: "staleness", value: 12, timestamp: Date.now() },
            { contract: "treasury", metric: "balance", value: 1000000, timestamp: Date.now() },
        ];
        
        // Format metrics as JSON
        const payload = JSON.stringify({
            dashboard: HEALTH_DASHBOARD_ID,
            timestamp: Date.now(),
            metrics: mockResponse
        }, null, 2);
        
        console.log("Exporting metrics:\n", payload);
        
        // Push to monitoring endpoint
        try {
            await fetch(MONITORING_ENDPOINT, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: payload
            });
            console.log(`Successfully pushed metrics to ${MONITORING_ENDPOINT}`);
        } catch (postErr) {
            console.warn(`Failed to push to endpoint (endpoint may be offline): ${postErr.message}`);
        }
        
    } catch (err) {
        console.error("Error polling the Health Dashboard:", err);
    }
}

console.log(`Starting Node.js health monitor script. Polling every ${POLLING_INTERVAL / 1000} seconds.`);
setInterval(pollHealthDashboard, POLLING_INTERVAL);
pollHealthDashboard(); // Initial run
