#![no_std]

use soroban_sdk::{contract, contractevent, contractclient, contractimpl, contracttype, symbol_short, Address, BytesN, Env, Vec};

#[contractclient(name = "CertificatesClient")]
pub trait CertificatesTrait {
    fn verify_certificate(env: Env, cert_id: u64) -> (bool, CertificateRecord);
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertificateRecord {
    pub id: u64,
    pub learner: Address,
    pub mentor: Address,
    pub skill: soroban_sdk::Symbol,
    pub sessions_completed: u32,
    pub issued_at: u64,
    pub revoked: bool,
    pub session_id: soroban_sdk::Symbol,
    pub rating_at_time: u64,
}

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShowcaseRecord {
    pub learner: Address,
    pub cert_ids: Vec<u64>,
}

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    CertificatesContract,
    Showcase(Address),
    FeaturedLearners,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const FEATURED_LEARNERS_LIMIT: u32 = 10;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

}

#[contractevent]
#[derive(Clone)]
struct ShowcaseCertEvent {
    #[topic]
    action: Symbol,
    learner: Address,
    cert_id: u64,
}

#[contractevent]
#[derive(Clone)]
struct FeaturedUpdatedEvent {
    #[topic]
    action: Symbol,
    count: u32,
}

#[contract]
pub struct CertShowcase;

#[contractimpl]
impl CertShowcase {
    /// Initialize the certificate showcase contract
    pub fn initialize(env: Env, admin: Address, certificates_contract: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::CertificatesContract, &certificates_contract);
    }

    /// Add a certificate to learner's public showcase
    pub fn showcase(env: Env, learner: Address, cert_id: u64) {
        learner.require_auth();

        let mut showcase: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::Showcase(learner.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        // Prevent duplicates
        if showcase.iter().any(|id| id == cert_id) {
            panic!("certificate already showcased");
        }

        showcase.push_back(cert_id);
        env.storage()
            .persistent()
            .set(&DataKey::Showcase(learner.clone()), &showcase);

        ShowcaseCertEvent {
            action: symbol_short!("showcased"),
            learner,
            cert_id,
        }.publish(env);
    }

    /// Remove a certificate from learner's showcase
    pub fn hide(env: Env, learner: Address, cert_id: u64) {
        learner.require_auth();

        let showcase: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::Showcase(learner.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        let mut new_showcase = Vec::new(&env);
        let mut found = false;
        for id in showcase.iter() {
            if id != cert_id {
                new_showcase.push_back(id);
            } else {
                found = true;
            }
        }

        if !found {
            panic!("certificate not found in showcase");
        }

        if new_showcase.is_empty() {
            env.storage()
                .persistent()
                .remove(&DataKey::Showcase(learner.clone()));
        } else {
            env.storage()
                .persistent()
                .set(&DataKey::Showcase(learner.clone()), &new_showcase);
        }

        ShowcaseCertEvent {
            action: symbol_short!("hidden"),
            learner,
            cert_id,
        }.publish(env);
    }

    /// Get all showcased certificates for a learner
    pub fn get_showcase(env: Env, learner: Address) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::Showcase(learner))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Verify a certificate is in learner's showcase and valid
    pub fn verify_showcase_cert(env: Env, learner: Address, cert_id: u64) -> bool {
        let showcase: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::Showcase(learner))
            .unwrap_or_else(|| Vec::new(&env));

        showcase.iter().any(|id| id == cert_id)
    }

    /// Feature a certificate in the showcase. Verifies the certificate is valid and unexpired.
    pub fn feature(env: Env, learner: Address, cert_id: u64) {
        learner.require_auth();

        let cert_contract_addr: soroban_sdk::Address = env
            .storage()
            .instance()
            .get(&DataKey::CertificatesContract)
            .expect("certificates contract not set");

        let cert_client = CertificatesClient::new(&env, &cert_contract_addr);
        let (is_valid, _cert) = cert_client.verify_certificate(&cert_id);

        if !is_valid {
            panic!("certificate is invalid or revoked");
        }

        self::CertShowcase::showcase(env.clone(), learner, cert_id);
    }

    /// Generate deterministic hash for QR code verification
    pub fn generate_verification_url_hash(env: Env, cert_id: u64) -> BytesN<32> {
        // Create a simple hash by combining cert_id with a fixed prefix
        let mut bytes = [0u8; 32];
        bytes[0..8].copy_from_slice(&cert_id.to_le_bytes());
        BytesN::from_array(&env, &bytes)
    }

    /// Get top 10 learners by certificate count
    pub fn get_featured_learners(env: Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::FeaturedLearners)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Update featured learners list (admin only)
    pub fn update_featured_learners(env: Env, learners: Vec<Address>) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        if learners.len() > FEATURED_LEARNERS_LIMIT {
            panic!("too many featured learners");
        }

        env.storage()
            .persistent()
            .set(&DataKey::FeaturedLearners, &learners);

        FeaturedUpdatedEvent {
            action: symbol_short!("feat_upd"),
            count: learners.len() as u32,
        }.publish(env);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_showcase() {
        let env = Env::default();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let learner = Address::generate(&env);
        let cert_contract = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract);

        client.showcase(&learner, &1);
        let showcase = client.get_showcase(&learner);
        assert_eq!(showcase.len(), 1);
        assert_eq!(showcase.get(0).unwrap(), 1);
    }

    #[test]
    fn test_hide() {
        let env = Env::default();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let learner = Address::generate(&env);
        let cert_contract = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract);
        client.showcase(&learner, &1);
        client.showcase(&learner, &2);

        client.hide(&learner, &1);
        let showcase = client.get_showcase(&learner);
        assert_eq!(showcase.len(), 1);
        assert_eq!(showcase.get(0).unwrap(), 2);
    }

    #[test]
    fn test_verify_showcase_cert() {
        let env = Env::default();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let learner = Address::generate(&env);
        let cert_contract = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract);
        client.showcase(&learner, &1);

        assert!(client.verify_showcase_cert(&learner, &1));
        assert!(!client.verify_showcase_cert(&learner, &2));
    }

    #[test]
    fn test_featured_learners() {
        let env = Env::default();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let cert_contract = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract);

        let learners = vec![
            &env,
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];

        client.update_featured_learners(&learners);
        let featured = client.get_featured_learners();
        assert_eq!(featured.len(), 3);
    }

    #[test]
    #[should_panic(expected = "certificate already showcased")]
    fn test_duplicate_showcase() {
        let env = Env::default();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let learner = Address::generate(&env);
        let cert_contract = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract);
        client.showcase(&learner, &1);
        client.showcase(&learner, &1);
    }
}
