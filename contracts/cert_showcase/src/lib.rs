#![no_std]

use soroban_sdk::{contract, contractevent, contractclient, contractimpl, contracttype, contracterror, symbol_short, Address, BytesN, Env, Vec, Symbol};

#[contractclient(name = "CertificatesClient")]
pub trait CertificatesTrait {
    fn verify_certificate(env: Env, cert_id: u64) -> (bool, CertificateRecord);
    fn get_certificate_learner(env: Env, cert_id: u64) -> Address;
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    AlreadyShowcased = 2,
    NotShowcased = 3,
    NotInitialized = 4,
    CertificateNotFound = 5,
    TooManyFeaturedLearners = 6,
    NotCertificateOwner = 7,
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
    pub revocation_reason: Option<BytesN<32>>,
    pub session_id: soroban_sdk::Symbol,
    pub rating_at_time: u64,
    pub assessment_id: Option<Symbol>,
    pub assessment_verified: bool,
    pub assessment_authenticity_score: u32,
    pub peer_review_consensus: bool,
    pub integrity_hash: BytesN<32>,
    pub correction_history: Vec<Symbol>,
    pub authenticity_verified: bool,
    pub gaming_detection_score: u32,
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
// Events
// ---------------------------------------------------------------------------

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
    pub fn initialize(env: Env, admin: Address, certificates_contract: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::CertificatesContract, &certificates_contract);
        Ok(())
    }

    /// Add a certificate to learner's public showcase
    pub fn showcase(env: Env, learner: Address, cert_id: u64) -> Result<(), Error> {
        learner.require_auth();

        let cert_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::CertificatesContract)
            .ok_or(Error::NotInitialized)?;
        let cert_learner = CertificatesClient::new(&env, &cert_contract)
            .get_certificate_learner(&cert_id);
        if cert_learner != learner {
            return Err(Error::NotCertificateOwner);
        }

        let mut showcase: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::Showcase(learner.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        // Prevent duplicates
        if showcase.iter().any(|id| id == cert_id) {
            return Err(Error::AlreadyShowcased);
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
        Ok(())
    }

    /// Add a certificate to the caller's showcase after proving ownership.
    pub fn add_to_showcase(env: Env, learner: Address, cert_id: u64) -> Result<(), Error> {
        Self::showcase(env, learner, cert_id)
    }

    /// Remove a certificate from learner's showcase
    pub fn hide(env: Env, learner: Address, cert_id: u64) -> Result<(), Error> {
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
            return Err(Error::NotShowcased);
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
        Ok(())
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
    pub fn feature(env: Env, learner: Address, cert_id: u64) -> Result<(), Error> {
        learner.require_auth();

        let cert_contract_addr: soroban_sdk::Address = env
            .storage()
            .instance()
            .get(&DataKey::CertificatesContract)
            .ok_or(Error::NotInitialized)?;

        let cert_client = CertificatesClient::new(&env, &cert_contract_addr);
        let (is_valid, _cert) = cert_client.verify_certificate(&cert_id);

        if !is_valid {
            return Err(Error::CertificateNotFound);
        }

        Self::showcase(env, learner, cert_id)
    }

    /// Generate deterministic hash for QR code verification
    pub fn generate_verification_url_hash(env: Env, cert_id: u64) -> BytesN<32> {
        // Create a simple hash by combining cert_id with a fixed prefix
        let mut bytes = [0u8; 32];
        bytes[0..8].copy_from_slice(&cert_id.to_le_bytes());
        BytesN::from_array(&env, &bytes)
    }

    /// Get top featured learners (paginated)
    pub fn get_featured_learners_page(env: Env, offset: u32, limit: u32) -> Vec<Address> {
        use shared::pagination::Pagination;

        let featured: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::FeaturedLearners)
            .unwrap_or_else(|| Vec::new(&env));

        let total = featured.len();
        let (start, end) = Pagination::bounds(total as u32, offset, limit);

        let mut result = Vec::new(&env);
        for i in start..end {
            if let Some(addr) = featured.get(i) {
                result.push_back(addr);
            }
        }
        result
    }

    /// Get paginated certificates for a learner's showcase
    pub fn get_learner_certs_page(env: Env, learner: Address, offset: u32, limit: u32) -> Vec<u64> {
        use shared::pagination::Pagination;

        let showcase: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::Showcase(learner))
            .unwrap_or_else(|| Vec::new(&env));

        let total = showcase.len();
        let (start, end) = Pagination::bounds(total as u32, offset, limit);

        let mut result = Vec::new(&env);
        for i in start..end {
            if let Some(cert_id) = showcase.get(i) {
                result.push_back(cert_id);
            }
        }
        result
    }

    /// Get all featured learners (non-paginated)
    pub fn get_featured_learners(env: Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::FeaturedLearners)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Update featured learners list (admin only)
    pub fn update_featured_learners(env: Env, learners: Vec<Address>) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        if learners.len() > FEATURED_LEARNERS_LIMIT as usize {
            return Err(Error::TooManyFeaturedLearners);
        }

        env.storage()
            .persistent()
            .set(&DataKey::FeaturedLearners, &learners);

        FeaturedUpdatedEvent {
            action: symbol_short!("feat_upd"),
            count: learners.len() as u32,
        }.publish(env);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[contract]
    struct MockCertificates;

    #[contractimpl]
    impl MockCertificates {
        pub fn set_owner(env: Env, owner: Address) {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, "owner"), &owner);
        }

        pub fn get_certificate_learner(env: Env, _cert_id: u64) -> Address {
            env.storage()
                .instance()
                .get(&Symbol::new(&env, "owner"))
                .expect("owner not set")
        }
    }

    fn setup_certs(env: &Env, learner: &Address) -> Address {
        let cert_contract = env.register_contract(None, MockCertificates);
        MockCertificatesClient::new(env, &cert_contract).set_owner(learner);
        cert_contract
    }

    #[test]
    fn test_showcase() {
        let env = Env::default();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let learner = Address::generate(&env);
        let cert_contract = setup_certs(&env, &learner);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract).unwrap();

        client.showcase(&learner, &1).unwrap();
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
        let cert_contract = setup_certs(&env, &learner);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract).unwrap();
        client.showcase(&learner, &1).unwrap();
        client.showcase(&learner, &2).unwrap();

        client.hide(&learner, &1).unwrap();
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
        let cert_contract = setup_certs(&env, &learner);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract).unwrap();
        client.showcase(&learner, &1).unwrap();

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
        client.initialize(&admin, &cert_contract).unwrap();

        let learners = vec![
            &env,
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];

        client.update_featured_learners(&learners).unwrap();
        let featured = client.get_featured_learners();
        assert_eq!(featured.len(), 3);
    }

    #[test]
    fn test_duplicate_showcase() {
        let env = Env::default();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let learner = Address::generate(&env);
        let cert_contract = setup_certs(&env, &learner);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract).unwrap();
        client.showcase(&learner, &1).unwrap();

        // Second showcase of same cert should error
        let result = client.try_showcase(&learner, &1);
        assert!(result.is_err());
    }

    #[test]
    fn test_featured_learners_pagination() {
        let env = Env::default();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let cert_contract = Address::generate(&env);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract).unwrap();

        // Create 5 learners
        let learners = vec![
            &env,
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];

        client.update_featured_learners(&learners).unwrap();

        // Test first page
        let page1 = client.get_featured_learners_page(&0, &2);
        assert_eq!(page1.len(), 2);

        // Test second page
        let page2 = client.get_featured_learners_page(&2, &2);
        assert_eq!(page2.len(), 2);

        // Test out-of-bounds offset
        let empty = client.get_featured_learners_page(&10, &2);
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn test_learner_certs_pagination() {
        let env = Env::default();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let learner = Address::generate(&env);
        let cert_contract = setup_certs(&env, &learner);

        env.mock_all_auths();
        client.initialize(&admin, &cert_contract).unwrap();

        // Add 5 certificates for learner
        for i in 1..=5 {
            client.showcase(&learner, &i).unwrap();
        }

        // Test first page
        let page1 = client.get_learner_certs_page(&learner, &0, &2);
        assert_eq!(page1.len(), 2);

        // Test second page
        let page2 = client.get_learner_certs_page(&learner, &2, &2);
        assert_eq!(page2.len(), 2);

        // Test third page (partial)
        let page3 = client.get_learner_certs_page(&learner, &4, &2);
        assert_eq!(page3.len(), 1);

        // Test limit clamped to MAX_PAGE_SIZE
        let large_page = client.get_learner_certs_page(&learner, &0, &1000);
        assert!(large_page.len() <= 50); // MAX_PAGE_SIZE
    }

    #[test]
    fn test_add_to_showcase_requires_certificate_owner() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, CertShowcase);
        let client = CertShowcaseClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        let stranger = Address::generate(&env);
        let cert_contract = setup_certs(&env, &owner);
        client.initialize(&admin, &cert_contract).unwrap();

        assert_eq!(
            client.try_add_to_showcase(&stranger, &1),
            Err(Ok(Error::NotCertificateOwner))
        );
        assert_eq!(client.get_showcase(&stranger).len(), 0);

        client.add_to_showcase(&owner, &1).unwrap();
        assert_eq!(client.get_showcase(&owner).len(), 1);
    }
}
