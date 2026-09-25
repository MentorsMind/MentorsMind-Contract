#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, Address, BytesN, Env, Symbol,
};

const MONTH_SECS: u64 = 30 * 24 * 60 * 60;
pub const MAX_MISSED_PAYMENTS: u32 = 3;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompletionReason {
    None,
    CapReached,
    DurationExpired,
    Defaulted,
}

/// Oracle contract interface used to verify income attestations before
/// a repayment is accepted. The oracle is the source of truth for
/// self-reported income, preventing learners from under-reporting to
/// reduce their share payments.
#[contractclient(name = "IncomeOracleClient")]
pub trait IncomeOracleTrait {
    fn verify_income_attestation(
        env: Env,
        learner: Address,
        period_id: u32,
        attested_income: i128,
        attestation_proof: BytesN<32>,
    ) -> bool;
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
    pub missed_payments: u32,
    pub last_period_id: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IsaStatus {
    Active,
    Completed,
    Expired,
    Defaulted,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    NextIsaId,
    Isa(u32),
    IncomeOracle,
    Admin,
    ProcessedPeriod(u32, u32),
    IsaStatus(u32),
    IsaPaymentSchedule(u32),
}

#[contract]
pub struct ISAContract;

#[contractimpl]
impl ISAContract {
    /// Set the admin and income oracle contract address. Admin only, one-time.
    pub fn initialize(env: Env, admin: Address, income_oracle: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::IncomeOracle, &income_oracle);
    }

    pub fn set_income_oracle(env: Env, admin: Address, income_oracle: Address) {
        Self::require_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::IncomeOracle, &income_oracle);
    }

    pub fn check_expiry(env: Env, isa_id: u32) -> bool {
        let mut isa: ISARecord = match env.storage().persistent().get(&DataKey::Isa(isa_id)) {
            Some(record) => record,
            None => panic!("ISA record not found"),
        };

        if !isa.active {
            return false;
        }

        let now = env.ledger().timestamp();
        if now >= isa.expires_at {
            isa.active = false;
            isa.completion_reason = CompletionReason::DurationExpired;
            env.storage().persistent().set(&DataKey::Isa(isa_id), &isa);
            env.storage().persistent().set(&DataKey::IsaStatus(isa_id), &IsaStatus::Expired);
            return true;
        }
        false
    }

    pub fn get_remaining_obligation(env: Env, isa_id: u32) -> i128 {
        let isa: ISARecord = match env.storage().persistent().get(&DataKey::Isa(isa_id)) {
            Some(record) => record,
            None => return 0,
        };

        if !isa.active || isa.completion_reason != CompletionReason::None {
            return 0;
        }

        let cap_amount = isa.funded_amount
            .checked_mul(isa.cap_multiple as i128)
            .expect("cap overflow");
        cap_amount.saturating_sub(isa.total_shared).max(0)
    }

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
            missed_payments: 0,
            last_period_id: 0,
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

    /// Record a repayment for a given period, verified against the income oracle.
    /// Rejects the payment if the oracle cannot confirm the attestation, if the
    /// period was already processed, or if the ISA is not active.
    pub fn record_payment(
        env: Env,
        learner: Address,
        isa_id: u32,
        period_id: u32,
        attested_income: i128,
        attestation_proof: BytesN<32>,
    ) {
        learner.require_auth();

        let mut isa: ISARecord = env
            .storage()
            .persistent()
            .get(&DataKey::Isa(isa_id))
            .expect("isa not found");

        if isa.learner != learner {
            panic!("not the isa learner");
        }
        if !isa.active {
            panic!("isa inactive");
        }
        if attested_income <= 0 {
            panic!("invalid attested income");
        }

        let processed_key = DataKey::ProcessedPeriod(isa_id, period_id);
        if env.storage().persistent().has(&processed_key) {
            panic!("period already processed");
        }

        let oracle: Address = env
            .storage()
            .instance()
            .get(&DataKey::IncomeOracle)
            .expect("income oracle not configured");

        let verified = IncomeOracleClient::new(&env, &oracle).verify_income_attestation(
            &learner,
            &period_id,
            &attested_income,
            &attestation_proof,
        );
        if !verified {
            panic!("income attestation not verified");
        }

        env.storage().persistent().set(&processed_key, &true);
        isa.last_period_id = period_id;

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
            Self::complete_isa(&env, isa_id, &mut isa, "cap");
            return;
        }

        let share_due = attested_income
            .checked_mul(isa.share_pct as i128)
            .expect("share overflow")
            / 10_000;

        // Overpayment beyond the cap is rejected outright rather than silently truncated,
        // per acceptance criteria; the caller must not attempt to pay past the cap.
        if share_due > remaining_cap {
            panic!("payment exceeds remaining obligation cap");
        }

        if share_due > 0 {
            isa.total_shared = isa
                .total_shared
                .checked_add(share_due)
                .expect("shared overflow");
            isa.missed_payments = 0;

            env.events().publish(
                (
                    Symbol::new(&env, "payment_recorded"),
                    isa_id,
                    isa.funder.clone(),
                ),
                (isa.learner.clone(), period_id, attested_income, share_due),
            );
        }

        if isa.total_shared >= cap_amount {
            Self::complete_isa(&env, isa_id, &mut isa, "cap");
            return;
        }

        env.storage().persistent().set(&DataKey::Isa(isa_id), &isa);
    }

    /// Admin-callable: mark a missed payment period for an ISA (e.g. when a
    /// learner fails to submit a payment for an expected period). Transitions
    /// the ISA to Defaulted once MAX_MISSED_PAYMENTS is exceeded.
    pub fn record_missed_payment(env: Env, admin: Address, isa_id: u32) {
        Self::require_admin(&env, &admin);

        let mut isa: ISARecord = env
            .storage()
            .persistent()
            .get(&DataKey::Isa(isa_id))
            .expect("isa not found");

        if !isa.active {
            panic!("isa inactive");
        }

        isa.missed_payments = isa.missed_payments.saturating_add(1);
        env.storage().persistent().set(&DataKey::Isa(isa_id), &isa);

        env.events().publish(
            (Symbol::new(&env, "payment_missed"), isa_id, isa.learner.clone()),
            isa.missed_payments,
        );
    }

    /// Admin-callable: check whether an ISA has exceeded the missed-payment
    /// threshold and, if so, transition it to Defaulted.
    pub fn check_default(env: Env, admin: Address, isa_id: u32) -> bool {
        Self::require_admin(&env, &admin);

        let mut isa: ISARecord = env
            .storage()
            .persistent()
            .get(&DataKey::Isa(isa_id))
            .expect("isa not found");

        if isa.active && isa.missed_payments > MAX_MISSED_PAYMENTS {
            isa.active = false;
            isa.completion_reason = CompletionReason::Defaulted;
            env.storage().persistent().set(&DataKey::Isa(isa_id), &isa);
            env.events().publish(
                (Symbol::new(&env, "isa_defaulted"), isa_id, isa.learner),
                isa.missed_payments,
            );
            return true;
        }

        false
    }

    /// Remaining obligation: cap_multiple * funded_amount - total_paid. Zero once capped.
    pub fn get_remaining_obligation(env: Env, isa_id: u32) -> i128 {
        let isa: ISARecord = env
            .storage()
            .persistent()
            .get(&DataKey::Isa(isa_id))
            .expect("isa not found");

        let cap_amount = Self::cap_amount(&isa);
        cap_amount.saturating_sub(isa.total_shared).max(0)
    }

    fn complete_isa(env: &Env, isa_id: u32, isa: &mut ISARecord, reason_tag: &str) {
        isa.active = false;
        isa.completion_reason = CompletionReason::CapReached;
        env.storage().persistent().set(&DataKey::Isa(isa_id), isa);
        env.events().publish(
            (
                Symbol::new(env, "isa_completed"),
                isa_id,
                isa.learner.clone(),
            ),
            (
                isa.funder.clone(),
                isa.total_shared,
                Symbol::new(env, reason_tag),
            ),
        );
    }

    fn cap_amount(isa: &ISARecord) -> i128 {
        isa.funded_amount
            .checked_mul(isa.cap_multiple as i128)
            .expect("cap overflow")
            / 100
    }

    fn require_admin(env: &Env, admin: &Address) {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        if stored_admin != *admin {
            panic!("admin address mismatch");
        }
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

#[cfg(test)]
mod oracle_tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{contract, contractimpl};

    #[contract]
    struct MockOracle;

    #[contracttype]
    #[derive(Clone)]
    enum MockOracleKey {
        Approve(Address, u32),
    }

    #[contractimpl]
    impl MockOracle {
        pub fn set_approved(env: Env, learner: Address, period_id: u32, approved: bool) {
            env.storage()
                .persistent()
                .set(&MockOracleKey::Approve(learner, period_id), &approved);
        }

        pub fn verify_income_attestation(
            env: Env,
            learner: Address,
            period_id: u32,
            _attested_income: i128,
            _attestation_proof: BytesN<32>,
        ) -> bool {
            env.storage()
                .persistent()
                .get(&MockOracleKey::Approve(learner, period_id))
                .unwrap_or(false)
        }
    }

    struct OracleFixture {
        env: Env,
        admin: Address,
        learner: Address,
        funder: Address,
        contract_id: Address,
        oracle_id: Address,
    }

    impl OracleFixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let learner = Address::generate(&env);
            let funder = Address::generate(&env);
            let contract_id = env.register_contract(None, ISAContract);
            let oracle_id = env.register_contract(None, MockOracle);

            let client = ISAContractClient::new(&env, &contract_id);
            client.initialize(&admin, &oracle_id);

            Self {
                env,
                admin,
                learner,
                funder,
                contract_id,
                oracle_id,
            }
        }

        fn client(&self) -> ISAContractClient<'_> {
            ISAContractClient::new(&self.env, &self.contract_id)
        }

        fn oracle(&self) -> MockOracleClient<'_> {
            MockOracleClient::new(&self.env, &self.oracle_id)
        }

        fn create_isa(&self) -> u32 {
            // 10% share, 2x cap, funded 1000 -> cap_amount = 2000.
            self.client()
                .create_isa(&self.learner, &self.funder, &1_000, &1_000, &200, &36)
        }

        fn proof(&self) -> BytesN<32> {
            BytesN::from_array(&self.env, &[7; 32])
        }
    }

    #[test]
    #[should_panic(expected = "income attestation not verified")]
    fn test_payment_without_valid_attestation_rejected() {
        let f = OracleFixture::setup();
        let isa_id = f.create_isa();

        // Oracle never approved this period.
        f.client()
            .record_payment(&f.learner, &isa_id, &1, &1_000, &f.proof());
    }

    #[test]
    #[should_panic(expected = "payment exceeds remaining obligation cap")]
    fn test_overpayment_beyond_cap_rejected() {
        let f = OracleFixture::setup();
        let isa_id = f.create_isa();

        // Attested income high enough that 10% share exceeds the 2000 cap in one shot.
        f.oracle().set_approved(&f.learner, &1, &true);
        f.client()
            .record_payment(&f.learner, &isa_id, &1, &50_000, &f.proof());
    }

    #[test]
    fn test_ten_monthly_payments_cap_enforced() {
        let f = OracleFixture::setup();
        let isa_id = f.create_isa();

        // 10 monthly payments at 10% income share of 1_000 income = 100/mo => 1000 total,
        // well under the 2000 cap; verifies total equals sum of shares.
        for period in 1..=10u32 {
            f.oracle().set_approved(&f.learner, &period, &true);
            f.client()
                .record_payment(&f.learner, &isa_id, &period, &1_000, &f.proof());
        }

        let isa = f.client().get_isa_status(&isa_id);
        assert_eq!(isa.total_shared, 1_000);
        assert!(isa.active);
        assert_eq!(f.client().get_remaining_obligation(&isa_id), 1_000);
    }

    #[test]
    fn test_cap_reached_auto_completes_isa() {
        let f = OracleFixture::setup();
        let isa_id = f.create_isa();

        // 10% of 20_000 = 2_000 = exactly the cap.
        f.oracle().set_approved(&f.learner, &1, &true);
        f.client()
            .record_payment(&f.learner, &isa_id, &1, &20_000, &f.proof());

        let isa = f.client().get_isa_status(&isa_id);
        assert_eq!(isa.total_shared, 2_000);
        assert!(!isa.active);
        assert_eq!(isa.completion_reason, CompletionReason::CapReached);
        assert_eq!(f.client().get_remaining_obligation(&isa_id), 0);
    }

    #[test]
    fn test_check_default_after_max_missed_payments() {
        let f = OracleFixture::setup();
        let isa_id = f.create_isa();

        for _ in 0..MAX_MISSED_PAYMENTS {
            f.client().record_missed_payment(&f.admin, &isa_id);
        }
        // At exactly MAX_MISSED_PAYMENTS, not yet defaulted (threshold is ">").
        assert!(!f.client().check_default(&f.admin, &isa_id));

        f.client().record_missed_payment(&f.admin, &isa_id);
        assert!(f.client().check_default(&f.admin, &isa_id));

        let isa = f.client().get_isa_status(&isa_id);
        assert!(!isa.active);
        assert_eq!(isa.completion_reason, CompletionReason::Defaulted);
    }

    #[test]
    fn test_successful_payment_resets_missed_counter() {
        let f = OracleFixture::setup();
        let isa_id = f.create_isa();

        f.client().record_missed_payment(&f.admin, &isa_id);
        f.client().record_missed_payment(&f.admin, &isa_id);

        f.oracle().set_approved(&f.learner, &1, &true);
        f.client()
            .record_payment(&f.learner, &isa_id, &1, &1_000, &f.proof());

        let isa = f.client().get_isa_status(&isa_id);
        assert_eq!(isa.missed_payments, 0);
    }
}
