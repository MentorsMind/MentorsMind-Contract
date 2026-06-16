import re
import os

def process_oracle():
    path = "contracts/oracle/src/lib.rs"
    with open(path, "r") as f:
        content = f.read()

    # 1. Add constant
    if "HEALTH_DASHBOARD" not in content:
        content = content.replace("const ADMIN: Symbol = symbol_short!(\"ADMIN\");", "const ADMIN: Symbol = symbol_short!(\"ADMIN\");\nconst HEALTH_DASHBOARD: Symbol = symbol_short!(\"HLTH_DB\");")

    # 2. Add set_health_dashboard
    set_dashboard = """
    pub fn set_health_dashboard(env: Env, dashboard: Address) {
        let admin: Address = env.storage().persistent().get(&ADMIN).unwrap();
        admin.require_auth();
        env.storage().persistent().set(&HEALTH_DASHBOARD, &dashboard);
    }
"""
    if "pub fn set_health_dashboard" not in content:
        content = content.replace("pub fn initialize(", set_dashboard + "\n    pub fn initialize(")

    # 3. Implement HealthReporter
    reporter = """
impl shared::HealthReporter for OracleContract {
    fn report_metric(env: soroban_sdk::Env, metric: soroban_sdk::Symbol, value: i128) {
        if let Some(dashboard) = env.storage().persistent().get::<_, soroban_sdk::Address>(&HEALTH_DASHBOARD) {
            let _ = env.try_invoke_contract::<(), soroban_sdk::Error>(
                &dashboard,
                &soroban_sdk::Symbol::new(&env, "receive_metric"),
                (soroban_sdk::Symbol::new(&env, "oracle"), metric, value).into_val(&env),
            );
        }
    }
}
"""
    if "impl shared::HealthReporter" not in content:
        content += reporter

    # 4. State changes: oracle staleness. We can push staleness timestamp in `update_price`? Or push 1 to trigger check
    inject = '\n        Self::report_metric(env.clone(), soroban_sdk::Symbol::new(&env, "staleness"), env.ledger().timestamp() as i128);\n'
    if inject not in content and "pub fn update_price" in content:
        content = re.sub(r'(pub fn update_price[\s\S]*?\{)', r'\1' + inject, content, count=1)

    with open(path, "w") as f:
        f.write(content)

process_oracle()
