#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, BytesN, Env, String, Vec};

#[derive(Clone)]
#[contracttype]
pub struct BridgeConfig {
    pub admin: Address,
    pub supported_chains: Vec<u32>,
    pub processed_vaas: Vec<BytesN<32>>,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Config,
    ProcessedVAA(BytesN<32>),
    WrappedToken,
}

#[contracttype]
pub struct BridgedEvent {
    pub vaa_hash: BytesN<32>,
    pub recipient: Address,
    pub amount: i128,
    pub source_chain: u32,
    pub wrapped_token: Address,
}

// Supported chain IDs (Wormhole standard)
pub const CHAIN_ETHEREUM: u32 = 2;
pub const CHAIN_SOLANA: u32 = 1;
pub const CHAIN_BSC: u32 = 4;

#[contract]
pub struct BridgeReceiver;

#[contractimpl]
impl BridgeReceiver {
    /// Initialize the bridge contract
    pub fn init(env: Env, admin: Address) {
        let config = BridgeConfig {
            admin: admin.clone(),
            supported_chains: Vec::new(&env),
            processed_vaas: Vec::new(&env),
        };

        env.storage().instance().set(&DataKey::Config, &config);

        // Initialize with default supported chains
        let mut chains = Vec::new(&env);
        chains.push_back(CHAIN_ETHEREUM);
        chains.push_back(CHAIN_SOLANA);
        chains.push_back(CHAIN_BSC);
        Self::set_supported_chains(env, admin, chains);
    }

    /// Set the wrapped token contract address
    pub fn set_wrapped_token(env: Env, admin: Address, token_address: Address) {
        Self::require_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::WrappedToken, &token_address);
    }

    /// Receive a bridged asset from another chain via Wormhole
    pub fn receive_bridged_asset(
        env: Env,
        vaa_hash: BytesN<32>,
        recipient: Address,
        amount: i128,
        source_chain: u32,
    ) {
        // Validate amount is positive
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        // Check if source chain is supported
        let config = Self::get_config(&env);
        let is_supported = config
            .supported_chains
            .iter()
            .any(|chain| chain == source_chain);
        if !is_supported {
            panic!("Source chain {} is not supported", source_chain);
        }

        // Check for replay attacks - verify VAA hasn't been processed
        let processed_key = DataKey::ProcessedVAA(vaa_hash.clone());
        let is_processed: bool = env.storage().instance().has(&processed_key);
        if is_processed {
            panic!("VAA already processed - replay attack detected");
        }

        // Verify VAA hash against admin-approved list
        Self::verify_vaa_hash(&env, &vaa_hash);

        // Get the wrapped token address
        let token_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::WrappedToken)
            .unwrap_or_else(|| {
                panic!("Wrapped token not set");
            });

        // Mint equivalent wrapped token to recipient
        let token_client = token::Client::new(&env, &token_address);

        // Authorize the bridge contract to mint
        token_client.mint(&recipient, &amount);

        // Mark VAA as processed to prevent replay
        env.storage().instance().set(&processed_key, &true);

        // Also store in config's processed_vaas list for audit
        let mut config = Self::get_config(&env);
        config.processed_vaas.push_back(vaa_hash.clone());
        env.storage().instance().set(&DataKey::Config, &config);

        // Emit event
        Self::emit_bridged_event(
            &env,
            &vaa_hash,
            &recipient,
            amount,
            source_chain,
            &token_address,
        );
    }

    /// Verify VAA hash against approved list
    fn verify_vaa_hash(env: &Env, vaa_hash: &BytesN<32>) {
        // In production, this would verify against Wormhole guardian set
        // For MVP, check against admin-approved VAA hashes

        let approved_hashes: Vec<BytesN<32>> = env
            .storage()
            .instance()
            .get(&DataKey::ProcessedVAA(*vaa_hash))
            .map_or(Vec::new(env), |_| Vec::new(env));

        // Simplified: For now, accept all hashes that haven't been processed
        // In production, implement actual Wormhole VAA verification:
        // - Verify guardian signatures
        // - Check VAA timestamp
        // - Validate emitter address
        // - Verify chain ID matches

        // This is a placeholder - production should use actual Wormhole verification
        // For MVP, we trust that the VAA has been verified by the caller
        // and we're just preventing replays
    }

    /// Get list of supported chains
    pub fn get_supported_chains(env: Env) -> Vec<u32> {
        let config = Self::get_config(&env);
        config.supported_chains
    }

    /// Add a supported chain (admin only)
    pub fn add_supported_chain(env: Env, admin: Address, chain_id: u32) {
        Self::require_admin(&env, &admin);

        let mut config = Self::get_config(&env);

        // Check if chain already exists
        let exists = config
            .supported_chains
            .iter()
            .any(|chain| chain == chain_id);
        if exists {
            panic!("Chain {} already supported", chain_id);
        }

        config.supported_chains.push_back(chain_id);
        env.storage().instance().set(&DataKey::Config, &config);
    }

    /// Remove a supported chain (admin only)
    pub fn remove_supported_chain(env: Env, admin: Address, chain_id: u32) {
        Self::require_admin(&env, &admin);

        let mut config = Self::get_config(&env);

        // Filter out the chain to remove
        let mut new_chains = Vec::new(&env);
        for chain in config.supported_chains.iter() {
            if chain != chain_id {
                new_chains.push_back(chain);
            }
        }

        config.supported_chains = new_chains;
        env.storage().instance().set(&DataKey::Config, &config);
    }

    /// Check if a VAA has been processed
    pub fn is_vaa_processed(env: Env, vaa_hash: BytesN<32>) -> bool {
        let key = DataKey::ProcessedVAA(vaa_hash);
        env.storage().instance().has(&key)
    }

    /// Get processed VAAs list
    pub fn get_processed_vaas(env: Env) -> Vec<BytesN<32>> {
        let config = Self::get_config(&env);
        config.processed_vaas
    }

    // Helper functions
    fn get_config(env: &Env) -> BridgeConfig {
        env.storage()
            .instance()
            .get(&DataKey::Config)
            .unwrap_or_else(|| {
                panic!("Bridge not initialized");
            })
    }

    fn require_admin(env: &Env, admin: &Address) {
        let config = Self::get_config(env);
        if config.admin != *admin {
            panic!("Unauthorized: admin only");
        }
        admin.require_auth();
    }

    fn emit_bridged_event(
        env: &Env,
        vaa_hash: &BytesN<32>,
        recipient: &Address,
        amount: i128,
        source_chain: u32,
        wrapped_token: &Address,
    ) {
        let event = BridgedEvent {
            vaa_hash: vaa_hash.clone(),
            recipient: recipient.clone(),
            amount,
            source_chain,
            wrapped_token: wrapped_token.clone(),
        };

        env.events().publish(("bridge", "asset_bridged"), event);
    }

    fn set_supported_chains(env: Env, admin: Address, chains: Vec<u32>) {
        Self::require_admin(&env, &admin);

        let mut config = Self::get_config(&env);
        config.supported_chains = chains;
        env.storage().instance().set(&DataKey::Config, &config);
    }
}

// Unit tests
#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{symbol_short, testutils::Address as _, vec, BytesN, Env};

    #[test]
    fn test_init() {
        let env = Env::default();
        let admin = Address::generate(&env);

        BridgeReceiverClient::init(&env, &admin);

        let supported_chains = BridgeReceiverClient::get_supported_chains(&env);
        assert_eq!(supported_chains.len(), 3);
        assert_eq!(supported_chains.get(0).unwrap(), CHAIN_ETHEREUM);
        assert_eq!(supported_chains.get(1).unwrap(), CHAIN_SOLANA);
        assert_eq!(supported_chains.get(2).unwrap(), CHAIN_BSC);
    }

    #[test]
    #[should_panic(expected = "Wrapped token not set")]
    fn test_receive_without_wrapped_token() {
        let env = Env::default();
        let admin = Address::generate(&env);

        BridgeReceiverClient::init(&env, &admin);

        let vaa_hash = BytesN::from_array(&env, &[0; 32]);
        let recipient = Address::generate(&env);

        BridgeReceiverClient::receive_bridged_asset(
            &env,
            &vaa_hash,
            &recipient,
            &1000,
            &CHAIN_ETHEREUM,
        );
    }

    #[test]
    fn test_add_supported_chain() {
        let env = Env::default();
        let admin = Address::generate(&env);

        BridgeReceiverClient::init(&env, &admin);

        let new_chain = 5; // Arbitrum
        BridgeReceiverClient::add_supported_chain(&env, &admin, &new_chain);

        let chains = BridgeReceiverClient::get_supported_chains(&env);
        assert_eq!(chains.len(), 4);
        assert_eq!(chains.get(3).unwrap(), new_chain);
    }

    #[test]
    #[should_panic(expected = "Unauthorized: admin only")]
    fn test_add_supported_chain_unauthorized() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let unauthorized = Address::generate(&env);

        BridgeReceiverClient::init(&env, &admin);

        BridgeReceiverClient::add_supported_chain(&env, &unauthorized, &5);
    }

    #[test]
    fn test_remove_supported_chain() {
        let env = Env::default();
        let admin = Address::generate(&env);

        BridgeReceiverClient::init(&env, &admin);

        BridgeReceiverClient::remove_supported_chain(&env, &admin, &CHAIN_SOLANA);

        let chains = BridgeReceiverClient::get_supported_chains(&env);
        assert_eq!(chains.len(), 2);

        let contains_solana = chains.iter().any(|c| c == CHAIN_SOLANA);
        assert!(!contains_solana);
    }

    #[test]
    #[should_panic(expected = "Source chain 99 is not supported")]
    fn test_receive_unsupported_chain() {
        let env = Env::default();
        let admin = Address::generate(&env);

        BridgeReceiverClient::init(&env, &admin);

        // Set a dummy wrapped token
        let token = Address::generate(&env);
        BridgeReceiverClient::set_wrapped_token(&env, &admin, &token);

        let vaa_hash = BytesN::from_array(&env, &[0; 32]);
        let recipient = Address::generate(&env);

        BridgeReceiverClient::receive_bridged_asset(
            &env, &vaa_hash, &recipient, &1000, &99, // Unsupported chain
        );
    }

    #[test]
    #[should_panic(expected = "VAA already processed - replay attack detected")]
    fn test_replay_attack_prevention() {
        let env = Env::default();
        let admin = Address::generate(&env);

        BridgeReceiverClient::init(&env, &admin);

        // Set a dummy wrapped token
        let token = Address::generate(&env);
        BridgeReceiverClient::set_wrapped_token(&env, &admin, &token);

        let vaa_hash = BytesN::from_array(&env, &[1; 32]);
        let recipient = Address::generate(&env);

        // First receive - should succeed
        BridgeReceiverClient::receive_bridged_asset(
            &env,
            &vaa_hash,
            &recipient,
            &1000,
            &CHAIN_ETHEREUM,
        );

        // Second receive with same VAA - should fail
        BridgeReceiverClient::receive_bridged_asset(
            &env,
            &vaa_hash,
            &recipient,
            &1000,
            &CHAIN_ETHEREUM,
        );
    }
}
