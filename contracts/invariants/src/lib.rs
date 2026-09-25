#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec,
};

/// Structured details for an invariant violation (Issue #753).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvariantViolation {
    pub contract: Symbol,
    pub invariant: Symbol,
    pub expected: i128,
    pub actual: i128,
}

/// Trait implemented by contracts or monitoring hooks for production invariant checking.
pub trait InvariantChecker {
    fn check_invariants(env: &Env) -> Vec<InvariantViolation>;
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    LastInvariantCheck(Symbol),
}

#[contract]
pub struct InvariantsContract;

#[contractimpl]
impl InvariantsContract {
    /// Execute all protocol invariant checks across Staking, Treasury, Insurance, and LendingPool (Issue #753).
    /// Returns an empty Vec when all invariants hold cleanly.
    pub fn check_all_invariants(
        env: Env,
        staking_total_staked: i128,
        staking_sum_stakes: i128,
        treasury_balance: i128,
        treasury_pending_allocations: i128,
        insurance_pool_balance: i128,
        insurance_claims_remaining: i128,
        lending_liquidity: i128,
        lending_net_balance: i128,
    ) -> Vec<InvariantViolation> {
        let mut violations = Vec::new(&env);
        let now = env.ledger().timestamp();

        // 1. Staking invariant: total_staked == sum(all_stake_records)
        if staking_total_staked != staking_sum_stakes {
            violations.push_back(InvariantViolation {
                contract: symbol_short!("staking"),
                invariant: symbol_short!("tot_stake"),
                expected: staking_total_staked,
                actual: staking_sum_stakes,
            });
        }
        env.storage()
            .persistent()
            .set(&DataKey::LastInvariantCheck(symbol_short!("staking")), &now);

        // 2. Treasury invariant: treasury_balance >= pending_allocations
        if treasury_balance < treasury_pending_allocations {
            violations.push_back(InvariantViolation {
                contract: symbol_short!("treasury"),
                invariant: symbol_short!("solvency"),
                expected: treasury_pending_allocations,
                actual: treasury_balance,
            });
        }
        env.storage()
            .persistent()
            .set(&DataKey::LastInvariantCheck(symbol_short!("treasury")), &now);

        // 3. Insurance invariant: insurance_pool_balance >= claims_remaining
        if insurance_pool_balance < insurance_claims_remaining {
            violations.push_back(InvariantViolation {
                contract: symbol_short!("insurance"),
                invariant: symbol_short!("coverage"),
                expected: insurance_claims_remaining,
                actual: insurance_pool_balance,
            });
        }
        env.storage()
            .persistent()
            .set(&DataKey::LastInvariantCheck(symbol_short!("insuran")), &now);

        // 4. Lending pool invariant: total_liquidity == deposits - loans
        if lending_liquidity != lending_net_balance {
            violations.push_back(InvariantViolation {
                contract: symbol_short!("lending"),
                invariant: symbol_short!("liquidity"),
                expected: lending_liquidity,
                actual: lending_net_balance,
            });
        }
        env.storage()
            .persistent()
            .set(&DataKey::LastInvariantCheck(symbol_short!("lending")), &now);

        violations
    }

    /// Read the timestamp of the last invariant check for a contract component.
    pub fn get_last_check_timestamp(env: Env, component: Symbol) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::LastInvariantCheck(component))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn test_check_all_invariants_clean() {
        let env = Env::default();
        let contract_id = env.register_contract(None, InvariantsContract);
        let client = InvariantsContractClient::new(&env, &contract_id);

        let violations = client.check_all_invariants(
            &1000, &1000, // Staking
            &5000, &2000, // Treasury
            &3000, &1500, // Insurance
            &4000, &4000, // Lending
        );

        assert_eq!(violations.len(), 0);
    }

    #[test]
    fn test_check_all_invariants_detects_violations() {
        let env = Env::default();
        let contract_id = env.register_contract(None, InvariantsContract);
        let client = InvariantsContractClient::new(&env, &contract_id);

        let violations = client.check_all_invariants(
            &1000, &800,  // Staking broken
            &1000, &2000, // Treasury insolvent
            &3000, &1500, // Insurance clean
            &4000, &4000, // Lending clean
        );

        assert_eq!(violations.len(), 2);
        assert_eq!(violations.get(0).unwrap().contract, symbol_short!("staking"));
        assert_eq!(violations.get(1).unwrap().contract, symbol_short!("treasury"));
    }
}
