import sys
import re

def modify_escrow():
    path = "escrow/src/lib.rs"
    with open(path, "r") as f:
        content = f.read()

    # 1. Add shared::HealthReporter to imports if needed
    if "use shared::HealthReporter;" not in content:
        content = content.replace("use soroban_sdk::{", "use shared::HealthReporter;\nuse soroban_sdk::{")
    
    # 2. Add HealthDashboard to DataKey
    if "HealthDashboard," not in content:
        content = content.replace("ApprovedToken(Address),", "ApprovedToken(Address),\n    HealthDashboard,")
    
    # 3. Add ACTIVE_ESCROWS Symbol
    if "ACTIVE_ESCROWS" not in content:
        content = content.replace("const ESCROW_COUNT: Symbol", "const ACTIVE_ESCROWS: Symbol = symbol_short!(\"ACT_ESC\");\nconst ESCROW_COUNT: Symbol")
    
    # 4. Implement HealthReporter for EscrowContract
    impl_reporter = """
impl HealthReporter for EscrowContract {
    fn report_metric(env: Env, metric: Symbol, value: i128) {
        if let Some(dashboard) = env.storage().persistent().get::<_, Address>(&DataKey::HealthDashboard) {
            let _ = env.try_invoke_contract::<(), soroban_sdk::Error>(
                &dashboard,
                &Symbol::new(&env, "receive_metric"),
                (Symbol::new(&env, "escrow"), metric, value).into_val(&env),
            );
        }
    }
}
"""
    if "impl HealthReporter for EscrowContract" not in content:
        content += impl_reporter
    
    # 5. Add set_health_dashboard to EscrowContract
    set_dashboard = """
    pub fn set_health_dashboard(env: Env, dashboard: Address) {
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage().persistent().set(&DataKey::HealthDashboard, &dashboard);
    }
"""
    if "pub fn set_health_dashboard" not in content:
        content = content.replace("pub fn initialize(", set_dashboard + "\n    pub fn initialize(")

    # 6. Update active escrows in create_escrow
    # Look for: env.events().publish((symbol_short!("escrow_c"),),
    create_inject = """
        let active_escrows: u32 = env.storage().persistent().get(&ACTIVE_ESCROWS).unwrap_or(0) + 1;
        env.storage().persistent().set(&ACTIVE_ESCROWS, &active_escrows);
        Self::report_metric(env.clone(), Symbol::new(&env, "active_count"), active_escrows as i128);
"""
    if "active_count" not in content and "escrow_c" in content:
        content = re.sub(r'(env\.events\(\)\.publish\(\(symbol_short!\("escrow_c"\),\),)', create_inject + r'\n        \1', content)

    # 7. Update active escrows in release, refund, resolve. It's safe to just decrement it.
    decrement_inject = """
        let active_escrows: u32 = env.storage().persistent().get(&ACTIVE_ESCROWS).unwrap_or(1).saturating_sub(1);
        env.storage().persistent().set(&ACTIVE_ESCROWS, &active_escrows);
        Self::report_metric(env.clone(), Symbol::new(&env, "active_count"), active_escrows as i128);
"""
    for evt in ["escrow_r", "escrow_rf", "escrow_rs"]:
        if decrement_inject not in content and evt in content:
             content = re.sub(rf'(env\.events\(\)\.publish\(\(symbol_short!\("{evt}"\),\),)', decrement_inject + r'\n        \1', content)

    with open(path, "w") as f:
        f.write(content)
    print("Modified escrow successfully.")

modify_escrow()
