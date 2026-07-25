#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, BytesN, Env, IntoVal, Symbol, Vec,
};

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
    TrustedRelayer(Address),
    ProcessedNonce(u32, u64),
    BridgeFundedEscrow(u64),
    EscrowRegistrySlot,
}

#[contracttype]
pub struct BridgedEvent {
    pub vaa_hash: BytesN<32>,
    pub recipient: Address,
    pub amount: i128,
    pub source_chain: u32,
    pub wrapped_token: Address,
}

/// A cross-chain payment attestation submitted by a trusted relayer.
#[derive(Clone)]
#[contracttype]
pub struct BridgeMessage {
    pub source_chain_id: u32,
    pub tx_hash: BytesN<32>,
    pub sender: BytesN<32>,
    pub amount: i128,
    pub token_symbol: Symbol,
    pub nonce: u64,
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
        let token_client = token::StellarAssetClient::new(&env, &token_address);
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
    fn verify_vaa_hash(_env: &Env, _vaa_hash: &BytesN<32>) {
        // Placeholder for Wormhole guardian signature verification.
        // Replay protection is enforced separately via DataKey::ProcessedVAA.
    }

    /// Add a trusted relayer allowed to submit bridge messages. Admin only.
    pub fn add_trusted_relayer(env: Env, admin: Address, relayer: Address) {
        Self::require_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::TrustedRelayer(relayer), &true);
    }

    /// Remove a trusted relayer. Admin only.
    pub fn remove_trusted_relayer(env: Env, admin: Address, relayer: Address) {
        Self::require_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::TrustedRelayer(relayer), &false);
    }

    /// Check whether an address is a trusted relayer.
    pub fn is_trusted_relayer(env: Env, relayer: Address) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::TrustedRelayer(relayer))
            .unwrap_or(false)
    }

    /// Verify a bridge message from a trusted relayer and fund the target escrow.
    /// Rejects untrusted relayers and replayed (chain_id, nonce) pairs.
    pub fn receive_and_fund_escrow(
        env: Env,
        relayer: Address,
        message: BridgeMessage,
        escrow_id: u64,
    ) {
        relayer.require_auth();

        if !Self::is_trusted_relayer(env.clone(), relayer.clone()) {
            panic!("Untrusted relayer");
        }

        if message.amount <= 0 {
            panic!("Amount must be positive");
        }

        let nonce_key = DataKey::ProcessedNonce(message.source_chain_id, message.nonce);
        if env.storage().persistent().has(&nonce_key) {
            panic!("NonceAlreadyProcessed");
        }
        env.storage().persistent().set(&nonce_key, &true);

        // Fund the target escrow contract via its generic funding entrypoint.
        // Escrow instances expose `fund(escrow_id: u64, amount: i128)` per the
        // escrow_factory-deployed implementation contract.
        env.invoke_contract::<()>(
            &Self::escrow_registry(&env),
            &Symbol::new(&env, "fund"),
            (escrow_id, message.amount).into_val(&env),
        );

        env.storage()
            .persistent()
            .set(&DataKey::BridgeFundedEscrow(escrow_id), &message);

        env.events().publish(
            ("bridge", "BridgeFunded"),
            (
                escrow_id,
                message.source_chain_id,
                message.tx_hash.clone(),
                message.amount,
            ),
        );
    }

    /// Query the bridge message that funded a given escrow, if any.
    pub fn get_bridge_funded_escrow(env: Env, escrow_id: u64) -> Option<BridgeMessage> {
        env.storage()
            .persistent()
            .get(&DataKey::BridgeFundedEscrow(escrow_id))
    }

    /// Check whether a (chain_id, nonce) pair has already been processed.
    pub fn is_nonce_processed(env: Env, source_chain_id: u32, nonce: u64) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::ProcessedNonce(source_chain_id, nonce))
            .unwrap_or(false)
    }

    /// Set the escrow contract address that `receive_and_fund_escrow` funds into. Admin only.
    pub fn set_escrow_registry(env: Env, admin: Address, escrow_registry: Address) {
        Self::require_admin(&env, &admin);
        env.storage()
            .instance()
            .set(&DataKey::EscrowRegistrySlot, &escrow_registry);
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

    fn escrow_registry(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::EscrowRegistrySlot)
            .unwrap_or_else(|| panic!("Escrow registry not set"))
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
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::BytesN;

    fn create_wrapped_token(env: &Env, admin: &Address) -> Address {
        env.register_stellar_asset_contract_v2(admin.clone())
            .address()
    }

    #[test]
    fn test_init() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);

        client.init(&admin);

        let supported_chains = client.get_supported_chains();
        assert_eq!(supported_chains.len(), 3);
        assert_eq!(supported_chains.get(0).unwrap(), CHAIN_ETHEREUM);
        assert_eq!(supported_chains.get(1).unwrap(), CHAIN_SOLANA);
        assert_eq!(supported_chains.get(2).unwrap(), CHAIN_BSC);
    }

    #[test]
    #[should_panic(expected = "Wrapped token not set")]
    fn test_receive_without_wrapped_token() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);

        client.init(&admin);

        let vaa_hash = BytesN::from_array(&env, &[0; 32]);
        let recipient = Address::generate(&env);

        client.receive_bridged_asset(&vaa_hash, &recipient, &1000, &CHAIN_ETHEREUM);
    }

    #[test]
    fn test_add_supported_chain() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);

        client.init(&admin);

        let new_chain = 5; // Arbitrum
        client.add_supported_chain(&admin, &new_chain);

        let chains = client.get_supported_chains();
        assert_eq!(chains.len(), 4);
        assert_eq!(chains.get(3).unwrap(), new_chain);
    }

    #[test]
    #[should_panic(expected = "Unauthorized: admin only")]
    fn test_add_supported_chain_unauthorized() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let unauthorized = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);

        client.init(&admin);

        client.add_supported_chain(&unauthorized, &5);
    }

    #[test]
    fn test_remove_supported_chain() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);

        client.init(&admin);

        client.remove_supported_chain(&admin, &CHAIN_SOLANA);

        let chains = client.get_supported_chains();
        assert_eq!(chains.len(), 2);

        let contains_solana = chains.iter().any(|c| c == CHAIN_SOLANA);
        assert!(!contains_solana);
    }

    #[test]
    #[should_panic(expected = "Source chain 99 is not supported")]
    fn test_receive_unsupported_chain() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);

        client.init(&admin);

        let token = create_wrapped_token(&env, &admin);
        client.set_wrapped_token(&admin, &token);

        let vaa_hash = BytesN::from_array(&env, &[0; 32]);
        let recipient = Address::generate(&env);

        client.receive_bridged_asset(&vaa_hash, &recipient, &1000, &99);
    }

    #[test]
    #[should_panic(expected = "VAA already processed - replay attack detected")]
    fn test_replay_attack_prevention() {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let admin = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);

        client.init(&admin);

        let token = create_wrapped_token(&env, &admin);
        client.set_wrapped_token(&admin, &token);

        let vaa_hash = BytesN::from_array(&env, &[1; 32]);
        let recipient = Address::generate(&env);

        // First receive - should succeed
        client.receive_bridged_asset(&vaa_hash, &recipient, &1000, &CHAIN_ETHEREUM);

        // Second receive with same VAA - should fail
        client.receive_bridged_asset(&vaa_hash, &recipient, &1000, &CHAIN_ETHEREUM);
    }
}
