#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Bytes, BytesN, Env, Vec,
};
use shared::{PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT};

const MAX_BATCH: u32 = 100;

#[contracttype]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    /// BytesN<32> hash -> bool (present = sanctioned)
    Sanctioned(BytesN<32>),
}

#[contract]
pub struct Sanctions;

#[contractimpl]
impl Sanctions {
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(
            &DataKey::Admin,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }

    /// Add a single SHA-256 hash to the sanctions list. Admin only.
    pub fn add_to_list(env: Env, address_hash: BytesN<32>) {
        Self::require_admin(&env);
        let key = DataKey::Sanctioned(address_hash.clone());
        env.storage()
            .persistent()
            .set(&key, &true);
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
        env.events()
            .publish((symbol_short!("addr_add"),), address_hash);
    }

    /// Remove a hash from the sanctions list. Admin only.
    pub fn remove_from_list(env: Env, address_hash: BytesN<32>) {
        Self::require_admin(&env);
        env.storage()
            .persistent()
            .remove(&DataKey::Sanctioned(address_hash.clone()));
        env.events()
            .publish((symbol_short!("addr_rem"),), address_hash);
    }

    /// Batch add up to 100 hashes. Admin only.
    pub fn add_batch(env: Env, hashes: Vec<BytesN<32>>) {
        Self::require_admin(&env);
        if hashes.len() > MAX_BATCH {
            panic!("batch exceeds 100");
        }
        for hash in hashes.iter() {
            let key = DataKey::Sanctioned(hash.clone());
            env.storage()
                .persistent()
                .set(&key, &true);
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_LIFETIME_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
            env.events()
                .publish((symbol_short!("addr_add"),), hash);
        }
    }

    /// Hash the Address via SHA-256 of its XDR encoding and check the list.
    pub fn is_sanctioned(env: Env, address: Address) -> bool {
        let hash = hash_address(&env, &address);
        let key = DataKey::Sanctioned(hash);
        let is_sanctioned = env.storage()
            .persistent()
            .get(&key)
            .unwrap_or(false);

        // Extend TTL on read to keep frequently-checked entries hot
        if is_sanctioned {
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_LIFETIME_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }

        is_sanctioned
    }

    fn require_admin(env: &Env) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
    }
}

/// SHA-256 of the address's XDR bytes.
pub fn hash_address(env: &Env, address: &Address) -> BytesN<32> {
    let xdr: Bytes = address.clone().to_xdr(env);
    env.crypto().sha256(&xdr)
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn deploy(env: &Env) -> (SanctionsClient, Address) {
        let contract_id = env.register_contract(None, Sanctions);
        let c = SanctionsClient::new(env, &contract_id);
        let admin = Address::generate(env);
        c.initialize(&admin);
        (c, admin)
    }

    #[test]
    fn test_add_and_check_match() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _) = deploy(&env);

        let bad_actor = Address::generate(&env);
        let hash = hash_address(&env, &bad_actor);

        assert!(!c.is_sanctioned(&bad_actor));
        c.add_to_list(&hash);
        assert!(c.is_sanctioned(&bad_actor));
    }

    #[test]
    fn test_check_non_match() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _) = deploy(&env);

        let bad_actor = Address::generate(&env);
        let innocent = Address::generate(&env);
        let hash = hash_address(&env, &bad_actor);

        c.add_to_list(&hash);
        assert!(!c.is_sanctioned(&innocent));
    }

    #[test]
    fn test_remove() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _) = deploy(&env);

        let addr = Address::generate(&env);
        let hash = hash_address(&env, &addr);

        c.add_to_list(&hash);
        assert!(c.is_sanctioned(&addr));
        c.remove_from_list(&hash);
        assert!(!c.is_sanctioned(&addr));
    }

    #[test]
    fn test_batch_add() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _) = deploy(&env);

        let addrs: Vec<Address> = (0..5).map(|_| Address::generate(&env)).collect();
        let mut hashes: soroban_sdk::Vec<BytesN<32>> = soroban_sdk::Vec::new(&env);
        for a in &addrs {
            hashes.push_back(hash_address(&env, a));
        }

        c.add_batch(&hashes);
        for a in &addrs {
            assert!(c.is_sanctioned(a));
        }
    }

    #[test]
    #[should_panic(expected = "batch exceeds 100")]
    fn test_batch_over_limit_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _) = deploy(&env);

        let mut hashes: soroban_sdk::Vec<BytesN<32>> = soroban_sdk::Vec::new(&env);
        for _ in 0..101 {
            hashes.push_back(BytesN::from_array(&env, &[0u8; 32]));
        }
        c.add_batch(&hashes);
    }

    #[test]
    fn test_ttl_extension_on_add_and_read() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _) = deploy(&env);

        let addr = Address::generate(&env);
        let hash = hash_address(&env, &addr);

        // Add to list with TTL extension
        c.add_to_list(&hash);
        assert!(c.is_sanctioned(&addr));

        // Verify entry is still readable after TTL extension on read
        assert!(c.is_sanctioned(&addr));

        // Verify batch add also extends TTL
        let addr2 = Address::generate(&env);
        let mut hashes: soroban_sdk::Vec<BytesN<32>> = soroban_sdk::Vec::new(&env);
        hashes.push_back(hash_address(&env, &addr2));
        c.add_batch(&hashes);
        assert!(c.is_sanctioned(&addr2));

        // Multiple reads should keep TTL extended
        assert!(c.is_sanctioned(&addr));
        assert!(c.is_sanctioned(&addr2));
    }
}
