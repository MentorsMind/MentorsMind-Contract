import re
import os

def process_staking():
    path = "contracts/staking/src/lib.rs"
    with open(path, "r") as f:
        content = f.read()

    # Add HealthDashboard to DataKey
    if "HealthDashboard" not in content:
        content = content.replace("PendingRewards(Address),", "PendingRewards(Address),\n    HealthDashboard,")

    # Add set_health_dashboard
    set_dashboard = """
    pub fn set_health_dashboard(env: Env, dashboard: Address) {
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage().persistent().set(&DataKey::HealthDashboard, &dashboard);
    }
"""
    if "pub fn set_health_dashboard" not in content:
        content = content.replace("pub fn initialize(", set_dashboard + "\n    pub fn initialize(")

    # Implement HealthReporter
    reporter = """
impl shared::HealthReporter for StakingContract {
    fn report_metric(env: soroban_sdk::Env, metric: soroban_sdk::Symbol, value: i128) {
        if let Some(dashboard) = env.storage().persistent().get::<_, soroban_sdk::Address>(&DataKey::HealthDashboard) {
            let _ = env.try_invoke_contract::<(), soroban_sdk::Error>(
                &dashboard,
                &soroban_sdk::Symbol::new(&env, "receive_metric"),
                (soroban_sdk::Symbol::new(&env, "staking"), metric, value).into_val(&env),
            );
        }
    }
}
"""
    if "impl shared::HealthReporter" not in content:
        content += reporter

    # State changes: tvl
    inject = '\n        Self::report_metric(env.clone(), soroban_sdk::Symbol::new(&env, "tvl"), 1);\n'
    if inject not in content and "env.events().publish((symbol_short!(\"staked\"), mentor.clone()), amount);" in content:
        content = content.replace("env.events().publish((symbol_short!(\"staked\"), mentor.clone()), amount);", "env.events().publish((symbol_short!(\"staked\"), mentor.clone()), amount);" + inject)

    with open(path, "w") as f:
        f.write(content)

process_staking()
