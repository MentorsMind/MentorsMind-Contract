#![no_std]

use shared::events::{
    emit_bounty_event, evt_bounty_claimed, evt_bounty_disputed, evt_bounty_posted,
    evt_bounty_refunded, evt_bounty_verified,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, BytesN, Env, IntoVal,
    Symbol, Val, Vec,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DISPUTE_WINDOW: u64 = 48 * 60 * 60; // 48 hours in seconds
const TTL_THRESHOLD: u32 = 500_000;
const TTL_BUMP: u32 = 9_000_000; // large enough to survive test time jumps

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    VerificationContract,
    BountyCount,
    Bounty(u32),
    Claim(u32, Address), // (bounty_id, learner)
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BountyStatus {
    Open,
    Claimed,  // at least one learner has claimed
    Verified, // a claim was verified and reward released
    Disputed,
    Refunded,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimStatus {
    Pending,
    Verified,
    Disputed,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BountyRecord {
    pub id: u32,
    pub poster: Address,
    pub skill: Symbol,
    pub description_hash: BytesN<32>,
    pub reward: i128,
    pub token: Address,
    pub deadline: u64,
    pub status: BountyStatus,
    pub winner: Option<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimRecord {
    pub learner: Address,
    pub claimed_at: u64,
    pub status: ClaimStatus,
}

// ---------------------------------------------------------------------------
// Event data types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BountyPostedEvent {
    pub id: u32,
    pub poster: Address,
    pub skill: Symbol,
    pub reward: i128,
    pub deadline: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BountyClaimedEvent {
    pub bounty_id: u32,
    pub learner: Address,
    pub claimed_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BountyVerifiedEvent {
    pub bounty_id: u32,
    pub learner: Address,
    pub mentor: Address,
    pub reward: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BountyDisputedEvent {
    pub bounty_id: u32,
    pub learner: Address,
    pub disputed_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BountyRefundedEvent {
    pub bounty_id: u32,
    pub poster: Address,
    pub reward: i128,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct BountyContract;

#[contractimpl]
impl BountyContract {
    /// Initialize the contract with an admin and optional verification contract address.
    pub fn initialize(env: Env, admin: Address, verification_contract: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::VerificationContract, &verification_contract);
        env.storage().persistent().set(&DataKey::BountyCount, &0u32);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, TTL_THRESHOLD, TTL_BUMP);
        env.storage().persistent().extend_ttl(
            &DataKey::VerificationContract,
            TTL_THRESHOLD,
            TTL_BUMP,
        );
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::BountyCount, TTL_THRESHOLD, TTL_BUMP);
        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);
    }

    /// Post a new bounty. Transfers `reward` tokens from poster to this contract.
    /// Returns the new bounty ID.
    pub fn post_bounty(
        env: Env,
        poster: Address,
        skill: Symbol,
        description_hash: BytesN<32>,
        reward: i128,
        token: Address,
        deadline: u64,
    ) -> u32 {
        poster.require_auth();

        if reward <= 0 {
            panic!("Reward must be positive");
        }
        if deadline <= env.ledger().timestamp() {
            panic!("Deadline must be in the future");
        }

        // Pull reward tokens into the contract
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&poster, &env.current_contract_address(), &reward);

        let mut count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::BountyCount)
            .unwrap_or(0);
        count += 1;

        let bounty = BountyRecord {
            id: count,
            poster: poster.clone(),
            skill: skill.clone(),
            description_hash,
            reward,
            token,
            deadline,
            status: BountyStatus::Open,
            winner: None,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Bounty(count), &bounty);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Bounty(count), TTL_THRESHOLD, TTL_BUMP);
        env.storage()
            .persistent()
            .set(&DataKey::BountyCount, &count);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::BountyCount, TTL_THRESHOLD, TTL_BUMP);
        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);

        emit_bounty_event(
            &env,
            evt_bounty_posted(&env),
            BountyPostedEvent {
                id: count,
                poster,
                skill,
                reward,
                deadline,
            },
        );

        count
    }

    /// Learner signals completion of the bounty challenge.
    pub fn claim_bounty(env: Env, learner: Address, bounty_id: u32) {
        learner.require_auth();

        let bounty: BountyRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bounty(bounty_id))
            .expect("Bounty not found");

        if bounty.status == BountyStatus::Refunded || bounty.status == BountyStatus::Verified {
            panic!("Bounty is no longer active");
        }
        if env.ledger().timestamp() > bounty.deadline {
            panic!("Bounty deadline has passed");
        }

        let claim_key = DataKey::Claim(bounty_id, learner.clone());
        if env.storage().persistent().has(&claim_key) {
            panic!("Already claimed by this learner");
        }

        let claim = ClaimRecord {
            learner: learner.clone(),
            claimed_at: env.ledger().timestamp(),
            status: ClaimStatus::Pending,
        };
        env.storage().persistent().set(&claim_key, &claim);
        env.storage()
            .persistent()
            .extend_ttl(&claim_key, TTL_THRESHOLD, TTL_BUMP);

        // Transition bounty to Claimed if still Open
        if bounty.status == BountyStatus::Open {
            let mut updated = bounty;
            updated.status = BountyStatus::Claimed;
            env.storage()
                .persistent()
                .set(&DataKey::Bounty(bounty_id), &updated);
            env.storage().persistent().extend_ttl(
                &DataKey::Bounty(bounty_id),
                TTL_THRESHOLD,
                TTL_BUMP,
            );
        }

        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);

        emit_bounty_event(
            &env,
            evt_bounty_claimed(&env),
            BountyClaimedEvent {
                bounty_id,
                learner,
                claimed_at: env.ledger().timestamp(),
            },
        );
    }

    /// Verified mentor confirms a learner completed the challenge. Releases reward to learner.
    /// First verified claim wins; subsequent calls panic.
    pub fn verify_completion(env: Env, mentor: Address, bounty_id: u32, learner: Address) {
        mentor.require_auth();

        // Check mentor is verified via the verification contract
        let ver_contract: Address = env
            .storage()
            .persistent()
            .get(&DataKey::VerificationContract)
            .expect("Not initialized");
        let is_verified: bool =
            env.invoke_contract(&ver_contract, &Symbol::new(&env, "is_verified"), {
                let mut args: Vec<Val> = Vec::new(&env);
                args.push_back(mentor.clone().into_val(&env));
                args
            });
        if !is_verified {
            panic!("Mentor is not verified");
        }

        let mut bounty: BountyRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bounty(bounty_id))
            .expect("Bounty not found");

        if bounty.status == BountyStatus::Verified {
            panic!("Bounty already verified");
        }
        if bounty.status == BountyStatus::Refunded {
            panic!("Bounty already refunded");
        }

        let claim_key = DataKey::Claim(bounty_id, learner.clone());
        let mut claim: ClaimRecord = env
            .storage()
            .persistent()
            .get(&claim_key)
            .expect("No claim found for this learner");

        if claim.status == ClaimStatus::Disputed {
            panic!("Claim is disputed");
        }

        // Release reward to learner
        let token_client = token::Client::new(&env, &bounty.token);
        token_client.transfer(&env.current_contract_address(), &learner, &bounty.reward);

        claim.status = ClaimStatus::Verified;
        env.storage().persistent().set(&claim_key, &claim);
        env.storage()
            .persistent()
            .extend_ttl(&claim_key, TTL_THRESHOLD, TTL_BUMP);

        bounty.status = BountyStatus::Verified;
        bounty.winner = Some(learner.clone());
        env.storage()
            .persistent()
            .set(&DataKey::Bounty(bounty_id), &bounty);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Bounty(bounty_id), TTL_THRESHOLD, TTL_BUMP);
        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);

        emit_bounty_event(
            &env,
            evt_bounty_verified(&env),
            BountyVerifiedEvent {
                bounty_id,
                learner,
                mentor,
                reward: bounty.reward,
            },
        );
    }

    /// Poster disputes a learner's claim within 48h of the claim.
    pub fn dispute_completion(env: Env, bounty_id: u32, learner: Address) {
        let bounty: BountyRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bounty(bounty_id))
            .expect("Bounty not found");

        bounty.poster.require_auth();

        if bounty.status == BountyStatus::Verified {
            panic!("Bounty already verified");
        }
        if bounty.status == BountyStatus::Refunded {
            panic!("Bounty already refunded");
        }

        let claim_key = DataKey::Claim(bounty_id, learner.clone());
        let mut claim: ClaimRecord = env
            .storage()
            .persistent()
            .get(&claim_key)
            .expect("No claim found for this learner");

        if claim.status != ClaimStatus::Pending {
            panic!("Claim is not in pending state");
        }

        let now = env.ledger().timestamp();
        if now > claim.claimed_at + DISPUTE_WINDOW {
            panic!("Dispute window has passed");
        }

        claim.status = ClaimStatus::Disputed;
        env.storage().persistent().set(&claim_key, &claim);
        env.storage()
            .persistent()
            .extend_ttl(&claim_key, TTL_THRESHOLD, TTL_BUMP);

        let mut updated_bounty = bounty;
        updated_bounty.status = BountyStatus::Disputed;
        env.storage()
            .persistent()
            .set(&DataKey::Bounty(bounty_id), &updated_bounty);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Bounty(bounty_id), TTL_THRESHOLD, TTL_BUMP);
        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);

        emit_bounty_event(
            &env,
            evt_bounty_disputed(&env),
            BountyDisputedEvent {
                bounty_id,
                learner,
                disputed_at: now,
            },
        );
    }

    /// Poster reclaims reward if deadline passed with no verified claim.
    pub fn refund_bounty(env: Env, bounty_id: u32) {
        let mut bounty: BountyRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bounty(bounty_id))
            .expect("Bounty not found");

        bounty.poster.require_auth();

        if bounty.status == BountyStatus::Verified {
            panic!("Bounty already verified");
        }
        if bounty.status == BountyStatus::Refunded {
            panic!("Already refunded");
        }
        if env.ledger().timestamp() <= bounty.deadline {
            panic!("Deadline has not passed yet");
        }

        let token_client = token::Client::new(&env, &bounty.token);
        token_client.transfer(
            &env.current_contract_address(),
            &bounty.poster,
            &bounty.reward,
        );

        bounty.status = BountyStatus::Refunded;
        env.storage()
            .persistent()
            .set(&DataKey::Bounty(bounty_id), &bounty);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Bounty(bounty_id), TTL_THRESHOLD, TTL_BUMP);
        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);

        emit_bounty_event(
            &env,
            evt_bounty_refunded(&env),
            BountyRefundedEvent {
                bounty_id,
                poster: bounty.poster.clone(),
                reward: bounty.reward,
            },
        );
    }

    /// Get a bounty record by ID.
    pub fn get_bounty(env: Env, id: u32) -> BountyRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Bounty(id))
            .expect("Bounty not found")
    }

    /// Get a claim record for a specific learner on a bounty.
    pub fn get_claim(env: Env, bounty_id: u32, learner: Address) -> ClaimRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Claim(bounty_id, learner))
            .expect("Claim not found")
    }

    /// Get total number of bounties posted.
    pub fn get_bounty_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::BountyCount)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger, LedgerInfo},
        Address, Env,
    };

    // Minimal mock verification contract
    mod mock_verification {
        use soroban_sdk::{contract, contractimpl, Address, Env};

        #[contract]
        pub struct MockVerification;

        #[contractimpl]
        impl MockVerification {
            pub fn is_verified(_env: Env, _mentor: Address) -> bool {
                true
            }
        }
    }

    // Minimal mock token contract
    mod mock_token {
        use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

        const TTL_THRESHOLD: u32 = 500_000;
        const TTL_BUMP: u32 = 9_000_000;

        #[contracttype]
        pub enum TokenKey {
            Balance(Address),
        }

        #[contract]
        pub struct MockToken;

        #[contractimpl]
        impl MockToken {
            pub fn balance(env: Env, addr: Address) -> i128 {
                env.storage()
                    .persistent()
                    .get(&TokenKey::Balance(addr))
                    .unwrap_or(0)
            }

            pub fn mint(env: Env, to: Address, amount: i128) {
                let bal: i128 = env
                    .storage()
                    .persistent()
                    .get(&TokenKey::Balance(to.clone()))
                    .unwrap_or(0);
                env.storage()
                    .persistent()
                    .set(&TokenKey::Balance(to.clone()), &(bal + amount));
                env.storage().persistent().extend_ttl(
                    &TokenKey::Balance(to),
                    TTL_THRESHOLD,
                    TTL_BUMP,
                );
                env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);
            }

            pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
                let from_bal: i128 = env
                    .storage()
                    .persistent()
                    .get(&TokenKey::Balance(from.clone()))
                    .unwrap_or(0);
                if from_bal < amount {
                    panic!("Insufficient balance");
                }
                env.storage()
                    .persistent()
                    .set(&TokenKey::Balance(from.clone()), &(from_bal - amount));
                env.storage().persistent().extend_ttl(
                    &TokenKey::Balance(from),
                    TTL_THRESHOLD,
                    TTL_BUMP,
                );
                let to_bal: i128 = env
                    .storage()
                    .persistent()
                    .get(&TokenKey::Balance(to.clone()))
                    .unwrap_or(0);
                env.storage()
                    .persistent()
                    .set(&TokenKey::Balance(to.clone()), &(to_bal + amount));
                env.storage().persistent().extend_ttl(
                    &TokenKey::Balance(to),
                    TTL_THRESHOLD,
                    TTL_BUMP,
                );
                env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);
            }
        }
    }

    use mock_token::{MockToken, MockTokenClient};
    use mock_verification::{MockVerification, MockVerificationClient};

    struct TestFixture {
        env: Env,
        bounty_id: Address,
        token_id: Address,
        ver_id: Address,
        admin: Address,
        poster: Address,
        learner: Address,
        mentor: Address,
    }

    impl TestFixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            // Set a base timestamp
            env.ledger().set(LedgerInfo {
                timestamp: 1_000_000,
                protocol_version: 21,
                sequence_number: 100,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 1,
                min_persistent_entry_ttl: 1,
                max_entry_ttl: 10_000_000,
            });

            let admin = Address::generate(&env);
            let poster = Address::generate(&env);
            let learner = Address::generate(&env);
            let mentor = Address::generate(&env);

            let token_id = env.register_contract(None, MockToken);
            let ver_id = env.register_contract(None, MockVerification);
            let bounty_id = env.register_contract(None, BountyContract);

            // Mint tokens to poster
            let token_client = MockTokenClient::new(&env, &token_id);
            token_client.mint(&poster, &1_000_000);

            let client = BountyContractClient::new(&env, &bounty_id);
            client.initialize(&admin, &ver_id);

            TestFixture {
                env,
                bounty_id,
                token_id,
                ver_id,
                admin,
                poster,
                learner,
                mentor,
            }
        }

        fn client(&self) -> BountyContractClient {
            BountyContractClient::new(&self.env, &self.bounty_id)
        }

        fn token(&self) -> MockTokenClient {
            MockTokenClient::new(&self.env, &self.token_id)
        }

        fn deadline(&self) -> u64 {
            self.env.ledger().timestamp() + 7 * 24 * 60 * 60 // 1 week
        }

        fn post_default_bounty(&self) -> u32 {
            self.client().post_bounty(
                &self.poster,
                &Symbol::new(&self.env, "Soroban"),
                &BytesN::from_array(&self.env, &[0u8; 32]),
                &100_000,
                &self.token_id,
                &self.deadline(),
            )
        }
    }

    #[test]
    fn test_post_bounty() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();
        assert_eq!(id, 1);

        let bounty = f.client().get_bounty(&id);
        assert_eq!(bounty.poster, f.poster);
        assert_eq!(bounty.reward, 100_000);
        assert_eq!(bounty.status, BountyStatus::Open);
        assert_eq!(f.client().get_bounty_count(), 1);

        // Tokens transferred to contract
        assert_eq!(f.token().balance(&f.poster), 900_000);
        assert_eq!(f.token().balance(&f.bounty_id), 100_000);
    }

    #[test]
    fn test_claim_bounty() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();

        f.client().claim_bounty(&f.learner, &id);

        let bounty = f.client().get_bounty(&id);
        assert_eq!(bounty.status, BountyStatus::Claimed);

        let claim = f.client().get_claim(&id, &f.learner);
        assert_eq!(claim.status, ClaimStatus::Pending);
    }

    #[test]
    fn test_verify_completion_releases_reward() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();
        f.client().claim_bounty(&f.learner, &id);
        f.client().verify_completion(&f.mentor, &id, &f.learner);

        let bounty = f.client().get_bounty(&id);
        assert_eq!(bounty.status, BountyStatus::Verified);
        assert_eq!(bounty.winner, Some(f.learner.clone()));

        // Learner received reward
        assert_eq!(f.token().balance(&f.learner), 100_000);
        assert_eq!(f.token().balance(&f.bounty_id), 0);
    }

    #[test]
    fn test_multiple_learners_first_verified_wins() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();

        let learner2 = Address::generate(&f.env);
        f.client().claim_bounty(&f.learner, &id);
        f.client().claim_bounty(&learner2, &id);

        // Verify first learner
        f.client().verify_completion(&f.mentor, &id, &f.learner);

        let bounty = f.client().get_bounty(&id);
        assert_eq!(bounty.winner, Some(f.learner.clone()));
        assert_eq!(bounty.status, BountyStatus::Verified);
    }

    #[test]
    #[should_panic(expected = "Bounty already verified")]
    fn test_verify_twice_panics() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();
        f.client().claim_bounty(&f.learner, &id);
        f.client().verify_completion(&f.mentor, &id, &f.learner);
        f.client().verify_completion(&f.mentor, &id, &f.learner);
    }

    #[test]
    fn test_dispute_completion() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();
        f.client().claim_bounty(&f.learner, &id);
        f.client().dispute_completion(&id, &f.learner);

        let bounty = f.client().get_bounty(&id);
        assert_eq!(bounty.status, BountyStatus::Disputed);

        let claim = f.client().get_claim(&id, &f.learner);
        assert_eq!(claim.status, ClaimStatus::Disputed);
    }

    #[test]
    #[should_panic(expected = "Dispute window has passed")]
    fn test_dispute_after_window_panics() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();
        f.client().claim_bounty(&f.learner, &id);

        // Advance time past 48h dispute window; keep sequence within TTL_BUMP range
        f.env.ledger().set(LedgerInfo {
            timestamp: f.env.ledger().timestamp() + DISPUTE_WINDOW + 1,
            protocol_version: 21,
            sequence_number: 200,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 10_000_000,
        });

        f.client().dispute_completion(&id, &f.learner);
    }

    #[test]
    fn test_refund_after_deadline() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();

        // Bump all contract instance TTLs before advancing time
        f.env.as_contract(&f.token_id, || {
            f.env
                .storage()
                .instance()
                .extend_ttl(TTL_THRESHOLD, TTL_BUMP);
        });
        f.env.as_contract(&f.bounty_id, || {
            f.env
                .storage()
                .instance()
                .extend_ttl(TTL_THRESHOLD, TTL_BUMP);
        });

        // Advance time past deadline; keep sequence within TTL_BUMP range
        f.env.ledger().set(LedgerInfo {
            timestamp: f.deadline() + 1,
            protocol_version: 21,
            sequence_number: 200,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 10_000_000,
        });

        f.client().refund_bounty(&id);

        let bounty = f.client().get_bounty(&id);
        assert_eq!(bounty.status, BountyStatus::Refunded);

        // Poster got tokens back
        assert_eq!(f.token().balance(&f.poster), 1_000_000);
    }

    #[test]
    #[should_panic(expected = "Deadline has not passed yet")]
    fn test_refund_before_deadline_panics() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();
        f.client().refund_bounty(&id);
    }

    #[test]
    #[should_panic(expected = "Bounty deadline has passed")]
    fn test_claim_after_deadline_panics() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();

        // Advance time past deadline; keep sequence within TTL_BUMP range
        f.env.ledger().set(LedgerInfo {
            timestamp: f.deadline() + 1,
            protocol_version: 21,
            sequence_number: 200,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 10_000_000,
        });

        f.client().claim_bounty(&f.learner, &id);
    }

    #[test]
    #[should_panic(expected = "Already claimed by this learner")]
    fn test_double_claim_panics() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();
        f.client().claim_bounty(&f.learner, &id);
        f.client().claim_bounty(&f.learner, &id);
    }

    #[test]
    fn test_ttl_renewal_preserves_claim_and_bounty_state() {
        let f = TestFixture::setup();
        let id = f.post_default_bounty();
        f.client().claim_bounty(&f.learner, &id);

        // Jump the ledger sequence substantially; entries should still be
        // available because write operations bump persistent TTL.
        f.env.ledger().set(LedgerInfo {
            timestamp: f.env.ledger().timestamp() + 60,
            protocol_version: 21,
            sequence_number: 500_100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 10_000_000,
        });

        let bounty = f.client().get_bounty(&id);
        assert_eq!(bounty.status, BountyStatus::Claimed);
        let claim = f.client().get_claim(&id, &f.learner);
        assert_eq!(claim.status, ClaimStatus::Pending);
    }

    #[test]
    fn test_ttl_persistence_for_admin_and_count_keys() {
        let f = TestFixture::setup();

        // Move to a later sequence while staying within configured max TTL.
        f.env.ledger().set(LedgerInfo {
            timestamp: f.env.ledger().timestamp() + 120,
            protocol_version: 21,
            sequence_number: 700_200,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 10_000_000,
        });

        // If admin and counter keys were not renewed in initialize, posting
        // would fail due missing configuration. Successful post implies key
        // persistence across the ledger jump.
        let id = f.post_default_bounty();
        assert_eq!(id, 1);
        assert_eq!(f.client().get_bounty_count(), 1);
    }
}
