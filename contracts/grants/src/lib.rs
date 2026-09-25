#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Bytes, BytesN, Env, String, Symbol, Vec};

// ── Storage keys ─────────────────────────────────────────────────────────────
const ADMIN: Symbol = symbol_short!("ADMIN");
const TREASURY: Symbol = symbol_short!("TREASURY");
const TTL_THRESHOLD: u32 = 500_000;
const TTL_BUMP: u32 = 1_000_000;

// ── Grant Economics ───────────────────────────────────────────────────────────
/// Maximum percentage of treasury that can be committed to grants (in basis points)
/// 2000 BPS = 20%
const MAX_GRANT_PCT_BPS: u16 = 2000;

// ── Types ─────────────────────────────────────────────────────────────────────
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantProgram {
    pub program_id: u32,
    pub budget: i128,
    pub allocated: i128,          // Amount allocated to learners
    pub per_learner_max: i128,
    pub eligibility_criteria_hash: BytesN<32>,
    pub expiry: u64,              // Timestamp when program expires
    pub sessions_funded: u32,     // Counter of funded sessions
    pub created_at: u64,
    pub token: Address,           // Token used for grants
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    /// Admin address
    GrantAdmin,
    /// Treasury contract address
    GrantTreasury,
    /// All grant programs: u32 -> GrantProgram
    GrantProgram(u32),
    /// Grant allocations per learner per program: (Address, u32) -> i128
    GrantAllocation(Address, u32),
    /// Eligibility proof storage: (Address, u32) -> BytesN<32>
    EligibilityProof(Address, u32),
    /// Total program count
    ProgramCount,
    /// Total committed to all grants
    TotalCommitted,
}

// ── Contract ──────────────────────────────────────────────────────────────────
#[contract]
pub struct Grants;

#[contractimpl]
impl Grants {
    /// Initialize grants contract with admin and treasury.
    pub fn initialize(env: Env, admin: Address, treasury: Address) {
        if env.storage().instance().has(&ADMIN) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&ADMIN, &admin);
        env.storage().instance().set(&TREASURY, &treasury);
        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);
    }

    /// Create a new grant program. Only callable by admin.
    /// Budget is approved from treasury via governance.
    pub fn create_grant_program(
        env: Env,
        admin: Address,
        budget: i128,
        per_learner_max: i128,
        eligibility_criteria_hash: BytesN<32>,
        expiry: u64,
        token: Address,
    ) -> u32 {
        Self::require_admin(&env, &admin);
        admin.require_auth();

        if budget <= 0 {
            panic!("Budget must be positive");
        }

        if per_learner_max <= 0 || per_learner_max > budget {
            panic!("Invalid per_learner_max");
        }

        if expiry <= env.ledger().timestamp() {
            panic!("Expiry must be in future");
        }

        // Check total committed doesn't exceed MAX_GRANT_PCT_BPS of treasury balance
        Self::check_grant_commitment(&env, &token, budget);

        let program_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProgramCount)
            .unwrap_or(0);

        let program_id = program_count;

        let program = GrantProgram {
            program_id,
            budget,
            allocated: 0,
            per_learner_max,
            eligibility_criteria_hash,
            expiry,
            sessions_funded: 0,
            created_at: env.ledger().timestamp(),
            token: token.clone(),
        };

        let program_key = DataKey::GrantProgram(program_id);
        env.storage().instance().set(&program_key, &program);

        // Update total committed
        let total_committed: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalCommitted)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalCommitted, &(total_committed + budget));

        // Update program count
        env.storage()
            .instance()
            .set(&DataKey::ProgramCount, &(program_count + 1));

        env.events().publish(
            (symbol_short!("grant"), Symbol::new(&env, "program_created")),
            program_id,
        );

        program_id
    }

    /// Apply for a grant. Learner submits eligibility proof.
    pub fn apply_for_grant(
        env: Env,
        learner: Address,
        program_id: u32,
        eligibility_proof: BytesN<32>,
    ) {
        learner.require_auth();

        let program_key = DataKey::GrantProgram(program_id);
        let program: GrantProgram = env
            .storage()
            .instance()
            .get(&program_key)
            .expect("Program not found");

        if env.ledger().timestamp() >= program.expiry {
            panic!("Program has expired");
        }

        // Store eligibility proof
        let proof_key = DataKey::EligibilityProof(learner.clone(), program_id);
        env.storage()
            .instance()
            .set(&proof_key, &eligibility_proof);

        env.events().publish(
            (symbol_short!("grant"), Symbol::new(&env, "grant_applied")),
            (learner, program_id),
        );
    }

    /// Approve a grant for a learner. Creates escrow funded from treasury.
    /// Only callable by admin. Amount must not exceed per_learner_max.
    pub fn approve_grant(
        env: Env,
        admin: Address,
        learner: Address,
        program_id: u32,
        amount: i128,
    ) {
        Self::require_admin(&env, &admin);
        admin.require_auth();

        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let program_key = DataKey::GrantProgram(program_id);
        let mut program: GrantProgram = env
            .storage()
            .instance()
            .get(&program_key)
            .expect("Program not found");

        if env.ledger().timestamp() >= program.expiry {
            panic!("Program has expired");
        }

        if amount > program.per_learner_max {
            panic!("Amount exceeds per_learner_max");
        }

        let allocation_key = DataKey::GrantAllocation(learner.clone(), program_id);
        let existing_allocation: i128 = env
            .storage()
            .instance()
            .get(&allocation_key)
            .unwrap_or(0);

        if existing_allocation > 0 {
            panic!("Learner already approved for this program");
        }

        // Check that total allocated doesn't exceed budget
        if program.allocated + amount > program.budget {
            panic!("Would exceed program budget");
        }

        // Verify eligibility proof was submitted
        let proof_key = DataKey::EligibilityProof(learner.clone(), program_id);
        if !env.storage().instance().has(&proof_key) {
            panic!("Learner has not applied for this program");
        }

        // Update program allocated
        program.allocated += amount;
        env.storage().instance().set(&program_key, &program);

        // Store learner allocation
        env.storage().instance().set(&allocation_key, &amount);

        env.events().publish(
            (symbol_short!("grant"), Symbol::new(&env, "grant_approved")),
            (learner.clone(), program_id, amount),
        );
    }

    /// Get a learner's grant allocation for a program.
    pub fn get_grant_allocation(env: Env, learner: Address, program_id: u32) -> i128 {
        let allocation_key = DataKey::GrantAllocation(learner, program_id);
        env.storage()
            .instance()
            .get(&allocation_key)
            .unwrap_or(0)
    }

    /// Get a grant program details.
    pub fn get_grant_program(env: Env, program_id: u32) -> GrantProgram {
        let program_key = DataKey::GrantProgram(program_id);
        env.storage()
            .instance()
            .get(&program_key)
            .expect("Program not found")
    }

    /// Get total number of grant programs created.
    pub fn get_program_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::ProgramCount)
            .unwrap_or(0)
    }

    /// Get total committed to all grants in basis points (as percentage of treasury).
    pub fn get_total_committed_bps(env: Env, token: Address) -> u16 {
        let treasury: Address = env
            .storage()
            .instance()
            .get(&TREASURY)
            .expect("Not initialized");

        let total_committed: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalCommitted)
            .unwrap_or(0);

        // Get treasury balance
        let treasury_balance = Self::get_token_balance(&env, &token, &treasury);

        if treasury_balance == 0 {
            return 0;
        }

        ((total_committed * 10000) / treasury_balance) as u16
    }

    /// Increment funded sessions counter for a program.
    /// Called when a learner uses grant-funded escrow for a session.
    pub fn increment_funded_sessions(env: Env, program_id: u32) {
        let program_key = DataKey::GrantProgram(program_id);
        let mut program: GrantProgram = env
            .storage()
            .instance()
            .get(&program_key)
            .expect("Program not found");

        program.sessions_funded += 1;
        env.storage().instance().set(&program_key, &program);

        env.events().publish(
            (symbol_short!("grant"), Symbol::new(&env, "session_funded")),
            (program_id, program.sessions_funded),
        );
    }

    // ── Helper methods ────────────────────────────────────────────────────────

    fn require_admin(env: &Env, admin: &Address) {
        let configured_admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN)
            .expect("Not initialized");

        if admin != &configured_admin {
            panic!("Unauthorized");
        }
    }

    fn check_grant_commitment(env: &Env, token: &Address, new_budget: i128) {
        let treasury: Address = env
            .storage()
            .instance()
            .get(&TREASURY)
            .expect("Not initialized");

        let total_committed: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalCommitted)
            .unwrap_or(0);

        let treasury_balance = Self::get_token_balance(env, token, &treasury);

        let max_allowed = (treasury_balance * (MAX_GRANT_PCT_BPS as i128)) / 10000;

        if total_committed + new_budget > max_allowed {
            panic!("Would exceed MAX_GRANT_PCT_BPS");
        }
    }

    fn get_token_balance(env: &Env, token: &Address, account: &Address) -> i128 {
        // Note: In a real implementation, this would call token contract's balance method
        // For now, returning a placeholder that would need treasury integration
        0
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000_000);

        let contract_id = env.register_contract(None, Grants);
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let token = Address::generate(&env);

        let client = GrantsClient::new(&env, &contract_id);
        client.initialize(&admin, &treasury);

        (env, admin, treasury, token)
    }

    fn dummy_hash(env: &Env) -> BytesN<32> {
        BytesN::from_array(env, &[1u8; 32])
    }

    #[test]
    fn test_create_grant_program() {
        let (env, admin, _treasury, token) = setup();

        let client = GrantsClient::new(
            &env,
            &env.register_contract(None, Grants),
        );

        // Re-initialize for this test
        let new_admin = Address::generate(&env);
        let new_treasury = Address::generate(&env);
        client.initialize(&new_admin, &new_treasury);

        let program_id = client.create_grant_program(
            &new_admin,
            &1000i128,              // budget
            &100i128,               // per_learner_max
            &dummy_hash(&env),
            &2_000_000u64,          // expiry
            &token,
        );

        assert_eq!(program_id, 0);

        let program = client.get_grant_program(&program_id);
        assert_eq!(program.budget, 1000);
        assert_eq!(program.per_learner_max, 100);
        assert_eq!(program.allocated, 0);
        assert_eq!(program.sessions_funded, 0);
    }

    #[test]
    fn test_apply_for_grant() {
        let (env, admin, _treasury, token) = setup();
        let learner = Address::generate(&env);

        let client = GrantsClient::new(
            &env,
            &env.register_contract(None, Grants),
        );

        let new_admin = Address::generate(&env);
        let new_treasury = Address::generate(&env);
        client.initialize(&new_admin, &new_treasury);

        let program_id = client.create_grant_program(
            &new_admin,
            &1000i128,
            &100i128,
            &dummy_hash(&env),
            &2_000_000u64,
            &token,
        );

        // Learner applies for grant
        client.apply_for_grant(
            &learner,
            &program_id,
            &dummy_hash(&env),
        );

        // Verify allocation is 0 before approval
        assert_eq!(client.get_grant_allocation(&learner, &program_id), 0);
    }

    #[test]
    fn test_approve_grant() {
        let (env, admin, _treasury, token) = setup();
        let learner = Address::generate(&env);

        let client = GrantsClient::new(
            &env,
            &env.register_contract(None, Grants),
        );

        let new_admin = Address::generate(&env);
        let new_treasury = Address::generate(&env);
        client.initialize(&new_admin, &new_treasury);

        let program_id = client.create_grant_program(
            &new_admin,
            &1000i128,
            &100i128,
            &dummy_hash(&env),
            &2_000_000u64,
            &token,
        );

        // Learner applies
        client.apply_for_grant(
            &learner,
            &program_id,
            &dummy_hash(&env),
        );

        // Admin approves grant
        client.approve_grant(
            &new_admin,
            &learner,
            &program_id,
            &50i128,
        );

        // Verify allocation
        assert_eq!(client.get_grant_allocation(&learner, &program_id), 50);

        // Verify program allocated increased
        let program = client.get_grant_program(&program_id);
        assert_eq!(program.allocated, 50);
    }

    #[test]
    #[should_panic(expected = "Learner already approved")]
    fn test_cannot_approve_twice() {
        let (env, admin, _treasury, token) = setup();
        let learner = Address::generate(&env);

        let client = GrantsClient::new(
            &env,
            &env.register_contract(None, Grants),
        );

        let new_admin = Address::generate(&env);
        let new_treasury = Address::generate(&env);
        client.initialize(&new_admin, &new_treasury);

        let program_id = client.create_grant_program(
            &new_admin,
            &1000i128,
            &100i128,
            &dummy_hash(&env),
            &2_000_000u64,
            &token,
        );

        client.apply_for_grant(&learner, &program_id, &dummy_hash(&env));

        client.approve_grant(&new_admin, &learner, &program_id, &50i128);
        client.approve_grant(&new_admin, &learner, &program_id, &50i128); // Should panic
    }

    #[test]
    #[should_panic(expected = "Learner has not applied")]
    fn test_cannot_approve_without_application() {
        let (env, admin, _treasury, token) = setup();
        let learner = Address::generate(&env);

        let client = GrantsClient::new(
            &env,
            &env.register_contract(None, Grants),
        );

        let new_admin = Address::generate(&env);
        let new_treasury = Address::generate(&env);
        client.initialize(&new_admin, &new_treasury);

        let program_id = client.create_grant_program(
            &new_admin,
            &1000i128,
            &100i128,
            &dummy_hash(&env),
            &2_000_000u64,
            &token,
        );

        // Try to approve without application
        client.approve_grant(&new_admin, &learner, &program_id, &50i128);
    }

    #[test]
    fn test_increment_funded_sessions() {
        let (env, admin, _treasury, token) = setup();
        let learner = Address::generate(&env);

        let client = GrantsClient::new(
            &env,
            &env.register_contract(None, Grants),
        );

        let new_admin = Address::generate(&env);
        let new_treasury = Address::generate(&env);
        client.initialize(&new_admin, &new_treasury);

        let program_id = client.create_grant_program(
            &new_admin,
            &1000i128,
            &100i128,
            &dummy_hash(&env),
            &2_000_000u64,
            &token,
        );

        let mut program = client.get_grant_program(&program_id);
        assert_eq!(program.sessions_funded, 0);

        client.increment_funded_sessions(&program_id);

        program = client.get_grant_program(&program_id);
        assert_eq!(program.sessions_funded, 1);
    }
}
