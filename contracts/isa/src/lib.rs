#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

const MONTH_SECS: u64 = 30 * 24 * 60 * 60;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompletionReason {
    None,
    CapReached,
    DurationExpired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ISARecord {
    pub isa_id: u32,
    pub learner: Address,
    pub funder: Address,
    pub funded_amount: i128,
    pub share_pct: u32,
    pub cap_multiple: u32,
    pub duration_months: u32,
    pub created_at: u64,
    pub expires_at: u64,
    pub total_shared: i128,
    pub active: bool,
    pub completion_reason: CompletionReason,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    NextIsaId,
    Isa(u32),
}

#[contract]
pub struct ISAContract;

#[contractimpl]
impl ISAContract {
    pub fn create_isa(
        env: Env,
        learner: Address,
        funder: Address,
        funded_amount: i128,
        share_pct: u32,
        cap_multiple: u32,
        duration_months: u32,
    ) -> u32 {
        if funded_amount <= 0 {
            panic!("invalid funded amount");
        }
        if share_pct == 0 || share_pct > 10_000 {
            panic!("invalid share pct");
        }
        if cap_multiple < 100 {
            panic!("invalid cap multiple");
        }
        if duration_months == 0 {
            panic!("invalid duration");
        }

        learner.require_auth();
        funder.require_auth();

        let next_id: u32 = env
            .storage()
            .instance()
            .get(&DataKey::NextIsaId)
            .unwrap_or(1);
        env.storage()
            .instance()
            .set(&DataKey::NextIsaId, &(next_id + 1));

        let now = env.ledger().timestamp();
        let expires_at = now.saturating_add((duration_months as u64).saturating_mul(MONTH_SECS));

        let record = ISARecord {
            isa_id: next_id,
            learner: learner.clone(),
            funder: funder.clone(),
            funded_amount,
            share_pct,
            cap_multiple,
            duration_months,
            created_at: now,
            expires_at,
            total_shared: 0,
            active: true,
            completion_reason: CompletionReason::None,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Isa(next_id), &record);

        env.events().publish(
            (Symbol::new(&env, "isa_created"), next_id, learner),
            (
                funder,
                funded_amount,
                share_pct,
                cap_multiple,
                duration_months,
            ),
        );

        next_id
    }

    pub fn record_earning(env: Env, isa_id: u32, earning: i128) {
        if earning <= 0 {
            panic!("invalid earning");
        }

        let mut isa: ISARecord = env
            .storage()
            .persistent()
            .get(&DataKey::Isa(isa_id))
            .expect("isa not found");

        if !isa.active {
            panic!("isa inactive");
        }

        let now = env.ledger().timestamp();
        if now >= isa.expires_at {
            isa.active = false;
            isa.completion_reason = CompletionReason::DurationExpired;
            env.storage().persistent().set(&DataKey::Isa(isa_id), &isa);
            env.events().publish(
                (Symbol::new(&env, "isa_completed"), isa_id, isa.learner),
                (isa.funder, isa.total_shared, Symbol::new(&env, "duration")),
            );
            return;
        }

        let cap_amount = Self::cap_amount(&isa);
        let remaining_cap = cap_amount.saturating_sub(isa.total_shared);

        if remaining_cap <= 0 {
            isa.active = false;
            isa.completion_reason = CompletionReason::CapReached;
            env.storage().persistent().set(&DataKey::Isa(isa_id), &isa);
            env.events().publish(
                (Symbol::new(&env, "isa_completed"), isa_id, isa.learner),
                (isa.funder, isa.total_shared, Symbol::new(&env, "cap")),
            );
            return;
        }

        let share_due = earning
            .checked_mul(isa.share_pct as i128)
            .expect("share overflow")
            / 10_000;

        let payout = if share_due > remaining_cap {
            remaining_cap
        } else {
            share_due
        };

        if payout > 0 {
            isa.total_shared = isa
                .total_shared
                .checked_add(payout)
                .expect("shared overflow");

            // Escrow can use this event amount to route funds to funder in settlement flow.
            env.events().publish(
                (
                    Symbol::new(&env, "earning_shared"),
                    isa_id,
                    isa.funder.clone(),
                ),
                (isa.learner.clone(), earning, payout),
            );
        }

        if isa.total_shared >= cap_amount {
            isa.active = false;
            isa.completion_reason = CompletionReason::CapReached;
            env.events().publish(
                (
                    Symbol::new(&env, "isa_completed"),
                    isa_id,
                    isa.learner.clone(),
                ),
                (
                    isa.funder.clone(),
                    isa.total_shared,
                    Symbol::new(&env, "cap"),
                ),
            );
        }

        env.storage().persistent().set(&DataKey::Isa(isa_id), &isa);
    }

    pub fn get_isa_status(env: Env, isa_id: u32) -> ISARecord {
        let mut isa: ISARecord = env
            .storage()
            .persistent()
            .get(&DataKey::Isa(isa_id))
            .expect("isa not found");

        if isa.active && env.ledger().timestamp() >= isa.expires_at {
            isa.active = false;
            isa.completion_reason = CompletionReason::DurationExpired;
        }

        isa
    }

    fn cap_amount(isa: &ISARecord) -> i128 {
        isa.funded_amount
            .checked_mul(isa.cap_multiple as i128)
            .expect("cap overflow")
            / 100
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    struct Fixture {
        env: Env,
        learner: Address,
        funder: Address,
        contract_id: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let learner = Address::generate(&env);
            let funder = Address::generate(&env);
            let contract_id = env.register_contract(None, ISAContract);

            Self {
                env,
                learner,
                funder,
                contract_id,
            }
        }

        fn client(&self) -> ISAContractClient<'_> {
            ISAContractClient::new(&self.env, &self.contract_id)
        }

        fn create_default_isa(&self) -> u32 {
            self.client()
                .create_isa(&self.learner, &self.funder, &1_000, &500, &200, &12)
        }
    }

    #[test]
    fn test_create_isa() {
        let f = Fixture::setup();
        let isa_id = f.create_default_isa();

        assert_eq!(isa_id, 1);

        let isa = f.client().get_isa_status(&isa_id);
        assert_eq!(isa.learner, f.learner);
        assert_eq!(isa.funder, f.funder);
        assert_eq!(isa.funded_amount, 1_000);
        assert_eq!(isa.share_pct, 500);
        assert_eq!(isa.cap_multiple, 200);
        assert_eq!(isa.duration_months, 12);
        assert_eq!(isa.total_shared, 0);
        assert!(isa.active);
        assert_eq!(isa.completion_reason, CompletionReason::None);
    }

    #[test]
    fn test_record_earning_shares_amount() {
        let f = Fixture::setup();
        let isa_id = f.create_default_isa();

        f.client().record_earning(&isa_id, &1_000);

        let isa = f.client().get_isa_status(&isa_id);
        assert_eq!(isa.total_shared, 50); // 5% of 1000
        assert!(isa.active);
        assert_eq!(isa.completion_reason, CompletionReason::None);
    }

    #[test]
    fn test_cap_reached_terminates_isa() {
        let f = Fixture::setup();
        let isa_id = f
            .client()
            .create_isa(&f.learner, &f.funder, &1_000, &10_000, &200, &12);

        f.client().record_earning(&isa_id, &1_500);
        let mid = f.client().get_isa_status(&isa_id);
        assert_eq!(mid.total_shared, 1_500);
        assert!(mid.active);

        f.client().record_earning(&isa_id, &600);
        let final_state = f.client().get_isa_status(&isa_id);
        assert_eq!(final_state.total_shared, 2_000); // capped at 2x funded_amount
        assert!(!final_state.active);
        assert_eq!(final_state.completion_reason, CompletionReason::CapReached);
    }

    #[test]
    fn test_duration_expiry_terminates_isa() {
        let f = Fixture::setup();
        let isa_id = f
            .client()
            .create_isa(&f.learner, &f.funder, &1_000, &1_000, &200, &1);

        f.env
            .ledger()
            .with_mut(|li| li.timestamp = MONTH_SECS.saturating_add(1));

        f.client().record_earning(&isa_id, &10_000);

        let isa = f.client().get_isa_status(&isa_id);
        assert_eq!(isa.total_shared, 0);
        assert!(!isa.active);
        assert_eq!(isa.completion_reason, CompletionReason::DurationExpired);
    }
}
