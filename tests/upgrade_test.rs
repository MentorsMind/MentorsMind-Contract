use mentorminds_escrow::{
    DataKey, Escrow, EscrowContract as EscrowContractV2, EscrowContractClient as EscrowV2Client,
    EscrowParams as EscrowParamsV2, EscrowStatus,
};
use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::Address as _,
    token,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Symbol, Vec,
};

const MAX_FEE_BPS: u32 = 1_000;
const DEFAULT_AUTO_RELEASE_DELAY: u64 = 72 * 60 * 60;
const ESCROW_TTL_THRESHOLD: u32 = 500_000;
const ESCROW_TTL_BUMP: u32 = 1_000_000;

#[contract]
struct EscrowContractV1;

#[contractimpl]
impl EscrowContractV1 {
    pub fn initialize(
        env: Env,
        admin: Address,
        treasury: Address,
        fee_bps: u32,
        approved_tokens: Vec<Address>,
        auto_release_delay_secs: u64,
    ) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        if fee_bps > MAX_FEE_BPS {
            panic!("Fee exceeds maximum (1000 bps)");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        env.storage()
            .persistent()
            .set(&DataKey::Treasury, &treasury);
        env.storage().persistent().extend_ttl(
            &DataKey::Treasury,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        env.storage().persistent().set(&DataKey::FeeBps, &fee_bps);
        env.storage().persistent().extend_ttl(
            &DataKey::FeeBps,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        env.storage().persistent().set(&DataKey::EscrowCount, &0u64);
        env.storage().persistent().extend_ttl(
            &DataKey::EscrowCount,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        let delay = if auto_release_delay_secs == 0 {
            DEFAULT_AUTO_RELEASE_DELAY
        } else {
            auto_release_delay_secs
        };
        env.storage()
            .persistent()
            .set(&DataKey::AutoRelDelay, &delay);
        env.storage().persistent().extend_ttl(
            &DataKey::AutoRelDelay,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        for token_addr in approved_tokens.iter() {
            let key = DataKey::ApprovedToken(token_addr.clone());
            env.storage().persistent().set(&key, &true);
            env.storage()
                .persistent()
                .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        }
    }

    pub fn create_escrow(
        env: Env,
        mentor: Address,
        learner: Address,
        amount: i128,
        session_id: Symbol,
        token_address: Address,
        session_end_time: u64,
    ) -> u64 {
        if amount <= 0 {
            panic!("Amount must be greater than zero");
        }
        if !env
            .storage()
            .persistent()
            .has(&DataKey::ApprovedToken(token_address.clone()))
        {
            panic!("Token not approved");
        }
        learner.require_auth();
        let token_client = token::Client::new(&env, &token_address);
        let learner_balance = token_client.balance(&learner);
        if learner_balance < amount {
            panic!("Insufficient token balance");
        }
        let auto_release_delay: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AutoRelDelay)
            .unwrap_or(DEFAULT_AUTO_RELEASE_DELAY);
        env.storage().persistent().extend_ttl(
            &DataKey::AutoRelDelay,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        let mut count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowCount)
            .unwrap_or(0);
        count += 1;
        env.storage()
            .persistent()
            .set(&DataKey::EscrowCount, &count);
        env.storage().persistent().extend_ttl(
            &DataKey::EscrowCount,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        token_client.transfer(&learner, &env.current_contract_address(), &amount);
        let escrow = Escrow {
            id: count,
            mentor: mentor.clone(),
            learner: learner.clone(),
            amount,
            session_id: session_id.clone(),
            status: EscrowStatus::Active,
            created_at: env.ledger().timestamp(),
            token_address: token_address.clone(),
            platform_fee: 0,
            net_amount: 0,
            session_end_time,
            auto_release_delay,
            dispute_reason: Symbol::new(&env, ""),
            resolved_at: 0,
            usd_amount: 0,
            quoted_token_amount: 0,
            send_asset: token_address.clone(),
            dest_asset: token_address,
            total_sessions: 1,
            sessions_completed: 0,
        };
        let key = DataKey::Escrow(count);
        env.storage().persistent().set(&key, &escrow);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        count
    }

    pub fn release_funds(env: Env, caller: Address, escrow_id: u64) {
        let key = DataKey::Escrow(escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");
        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Admin not found");
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        caller.require_auth();
        if caller != escrow.learner && caller != admin {
            panic!("Caller not authorized");
        }
        let fee_bps: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::FeeBps)
            .unwrap_or(0u32);
        env.storage().persistent().extend_ttl(
            &DataKey::FeeBps,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        let platform_fee: i128 = escrow.amount * (fee_bps as i128) / 10_000;
        let net_amount: i128 = escrow.amount - platform_fee;
        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not found");
        env.storage().persistent().extend_ttl(
            &DataKey::Treasury,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        let token_client = token::Client::new(&env, &escrow.token_address);
        if platform_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &platform_fee);
        }
        token_client.transfer(&env.current_contract_address(), &escrow.mentor, &net_amount);
        escrow.status = EscrowStatus::Released;
        escrow.platform_fee = platform_fee;
        escrow.net_amount = net_amount;
        env.storage().persistent().set(&key, &escrow);
    }

    pub fn refund(env: Env, escrow_id: u64) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Admin not found");
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        admin.require_auth();
        let key = DataKey::Escrow(escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");
        if matches!(
            escrow.status,
            EscrowStatus::Released | EscrowStatus::Refunded | EscrowStatus::Resolved
        ) {
            panic!("Cannot refund");
        }
        let token_client = token::Client::new(&env, &escrow.token_address);
        token_client.transfer(
            &env.current_contract_address(),
            &escrow.learner,
            &escrow.amount,
        );
        escrow.status = EscrowStatus::Refunded;
        env.storage().persistent().set(&key, &escrow);
    }

    pub fn get_escrow(env: Env, escrow_id: u64) -> Escrow {
        let key = DataKey::Escrow(escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found")
    }

    pub fn get_auto_release_delay(env: Env) -> u64 {
        env.storage().persistent().extend_ttl(
            &DataKey::AutoRelDelay,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        env.storage()
            .persistent()
            .get(&DataKey::AutoRelDelay)
            .unwrap_or(DEFAULT_AUTO_RELEASE_DELAY)
    }
}

fn create_token<'a>(env: &'a Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let token_address = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let sac = StellarAssetClient::new(env, &token_address);
    (token_address, sac)
}

#[test]
fn test_upgrade_path_preserves_storage_and_enables_new_features() {
    let env = Env::default();
    env.mock_all_auths();
    let fixed_id = Address::generate(&env);
    env.register_contract(Some(&fixed_id), EscrowContractV1);
    let v1 = EscrowContractV1Client::new(&env, &fixed_id);
    let admin = Address::generate(&env);
    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (token_address, sac) = create_token(&env, &admin);
    sac.mint(&learner, &10_000);
    let mut approved = Vec::new(&env);
    approved.push_back(token_address.clone());
    v1.initialize(&admin, &treasury, &500u32, &approved, &0u64);
    let now = env.ledger().timestamp();
    let _id2 = v1.create_escrow(
        &mentor,
        &learner,
        &1_000,
        &symbol_short!("S2"),
        &token_address,
        &now,
    );
    env.register_contract(Some(&fixed_id), EscrowContractV2);
    let v2 = EscrowV2Client::new(&env, &fixed_id);
    assert_eq!(v2.get_auto_release_delay(), 72 * 60 * 60);
    let token_client = TokenClient::new(&env, &token_address);
    let eid_new = v2.create_escrow(
        &mentor,
        &learner,
        &1_000,
        &symbol_short!("S3"),
        &token_address,
        &now,
        &1u32,
    );
    let before_mentor = token_client.balance(&mentor);
    let before_treasury = token_client.balance(&treasury);
    v2.release_funds(&learner, &eid_new);
    assert_eq!(token_client.balance(&mentor), before_mentor + 950);
    assert_eq!(token_client.balance(&treasury), before_treasury + 50);
    let reinit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let other = Address::generate(&env);
        let empty: Vec<Address> = Vec::new(&env);
        v2.initialize(&other, &treasury, &500u32, &empty, &0u64, &None);
    }));
    assert!(reinit.is_err());
}
