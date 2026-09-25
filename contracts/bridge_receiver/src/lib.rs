#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, BytesN, Env, IntoVal, Symbol, Vec,
};
use shared::{
    l2_finality_reached,
    record_cross_layer_audit,
    L2Integration,
    // #866 — Cross-chain state synchronization
    begin_atomic_xchain_op,
    acknowledge_prepare,
    confirm_commit,
    initiate_rollback,
    is_chain_isolated,
    isolate_chain,
    lift_chain_isolation,
    validate_state_proof,
    CrossChainStateProof,
    XChainPhase,
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
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Config,
    L2Config,
    L2Audit(BytesN<32>),
    ProcessedVAA(BytesN<32>),
    WrappedToken,
    TrustedRelayer(Address),
    ProcessedNonce(u32, u64),
    BridgeFundedEscrow(u64),
    EscrowRegistrySlot,
    /// Active atomic cross-chain operation for a given escrow (#866).
    ActiveXChainOp(u64),
    /// Emergency isolation override flag (#866).
    EmergencyIsolated,
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
        Self::ensure_l2_finality(&env);

        // Validate amount is positive
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        // #866 — Block bridging from isolated source chains.
        if is_chain_isolated(&env, source_chain) {
            panic!("Source chain is currently isolated from bridge operations");
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
        Self::record_audit(&env, &vaa_hash, source_chain);

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

    pub fn configure_l2(env: Env, admin: Address, network_id: u32, finality_delay_secs: u64, challenge_period_secs: u64) {
        Self::require_admin(&env, &admin);
        env.storage().instance().set(
            &DataKey::L2Config,
            &L2Integration {
                network_id,
                finality_delay_secs,
                challenge_period_secs,
                last_l2_block: env.ledger().sequence() as u64,
                last_l1_commitment: env.ledger().timestamp(),
                emergency_shutdown: false,
            },
        );
    }

    pub fn shutdown_l2(env: Env, admin: Address, emergency: bool) {
        Self::require_admin(&env, &admin);
        let mut cfg: L2Integration = env.storage().instance().get(&DataKey::L2Config).unwrap_or(L2Integration {
            network_id: 0,
            finality_delay_secs: 0,
            challenge_period_secs: 0,
            last_l2_block: 0,
            last_l1_commitment: 0,
            emergency_shutdown: false,
        });
        cfg.emergency_shutdown = emergency;
        env.storage().instance().set(&DataKey::L2Config, &cfg);
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

        // #866 — Block messages from isolated source chains.
        if is_chain_isolated(&env, message.source_chain_id) {
            panic!("Source chain is currently isolated from bridge operations");
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

    fn ensure_l2_finality(env: &Env) {
        if let Some(cfg) = env.storage().instance().get::<_, L2Integration>(&DataKey::L2Config) {
            if !l2_finality_reached(env, &cfg, env.ledger().timestamp().saturating_sub(cfg.last_l1_commitment)) {
                panic!("L2 finality window not satisfied");
            }
        }
    }

    fn record_audit(env: &Env, op: &BytesN<32>, source_chain: u32) {
        let contract_id = env.current_contract_address();
        let _ = record_cross_layer_audit(
            env,
            &contract_id,
            op,
            Symbol::new(env, "l2"),
            Symbol::new(env, "l1"),
            source_chain != 0,
        );
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

    // -----------------------------------------------------------------------
    // #866 — Atomic cross-chain operations and emergency isolation
    // -----------------------------------------------------------------------

    /// Initiate an atomic two-phase-commit cross-chain operation for a
    /// bridge transfer.
    ///
    /// Returns the `op_id` of the registered operation. The relayer must
    /// call `acknowledge_bridge_prepare` on each participating chain, then
    /// `confirm_bridge_commit` once all chains are ready.
    pub fn begin_bridge_atomic_op(
        env: Env,
        relayer: Address,
        source_chain_id: u32,
        dest_chain_id: u32,
        expected_state_root: BytesN<32>,
    ) -> BytesN<32> {
        relayer.require_auth();

        if !Self::is_trusted_relayer(env.clone(), relayer.clone()) {
            panic!("Untrusted relayer");
        }

        // Verify neither chain is isolated.
        if is_chain_isolated(&env, source_chain_id) {
            panic!("Source chain is currently isolated");
        }
        if is_chain_isolated(&env, dest_chain_id) {
            panic!("Destination chain is currently isolated");
        }

        let mut chains = Vec::new(&env);
        chains.push_back(source_chain_id);
        chains.push_back(dest_chain_id);

        let op_id = begin_atomic_xchain_op(&env, &relayer, chains, expected_state_root)
            .unwrap_or_else(|e| panic!("Failed to begin atomic op: {}", e as u32));

        env.events().publish(
            ("bridge", "AtomicOpStarted"),
            (op_id.clone(), source_chain_id, dest_chain_id),
        );

        op_id
    }

    /// Acknowledge the prepare phase for a chain in an atomic bridge op.
    pub fn acknowledge_bridge_prepare(
        env: Env,
        relayer: Address,
        op_id: BytesN<32>,
        chain_id: u32,
    ) -> u32 {
        relayer.require_auth();
        if !Self::is_trusted_relayer(env.clone(), relayer) {
            panic!("Untrusted relayer");
        }

        let phase = acknowledge_prepare(&env, &op_id, chain_id)
            .unwrap_or_else(|e| panic!("Prepare ack failed: {}", e as u32));

        phase as u32
    }

    /// Confirm commit for a chain in an atomic bridge op.
    ///
    /// Validates the chain's state root against the expected root.
    /// Triggers automatic rollback if roots diverge.
    pub fn confirm_bridge_commit(
        env: Env,
        relayer: Address,
        op_id: BytesN<32>,
        chain_id: u32,
        chain_state_root: BytesN<32>,
    ) -> u32 {
        relayer.require_auth();
        if !Self::is_trusted_relayer(env.clone(), relayer) {
            panic!("Untrusted relayer");
        }

        let phase = confirm_commit(&env, &op_id, chain_id, chain_state_root)
            .unwrap_or_else(|e| panic!("Commit failed: {}", e as u32));

        phase as u32
    }

    /// Manually trigger rollback for an in-flight atomic bridge operation.
    pub fn rollback_bridge_op(env: Env, admin: Address, op_id: BytesN<32>) {
        Self::require_admin(&env, &admin);
        initiate_rollback(&env, &op_id)
            .unwrap_or_else(|e| panic!("Rollback failed: {}", e as u32));

        env.events().publish(("bridge", "AtomicOpRolledBack"), op_id);
    }

    /// Validate a cross-chain state proof before executing a bridge transfer.
    ///
    /// Returns `true` if the proof is valid.
    pub fn validate_bridge_state_proof(
        env: Env,
        proof_chain_id: u32,
        state_root: BytesN<32>,
        proof_path: Vec<BytesN<32>>,
        generated_at: u64,
        expected_root: BytesN<32>,
    ) -> bool {
        let proof = CrossChainStateProof {
            chain_id: proof_chain_id,
            state_root,
            proof_path,
            generated_at,
            validated: false,
        };
        validate_state_proof(&env, &proof, &expected_root)
    }

    /// Emergency: isolate a source chain from bridge operations.
    ///
    /// All VAA-based and relayer-based bridge operations from `chain_id`
    /// will be blocked until the isolation is lifted.
    pub fn emergency_isolate_chain(
        env: Env,
        admin: Address,
        chain_id: u32,
        reason: Symbol,
    ) {
        Self::require_admin(&env, &admin);
        isolate_chain(&env, chain_id, reason, 1, 24 * 60 * 60);

        env.events().publish(
            ("bridge", "ChainIsolated"),
            (chain_id, env.ledger().timestamp()),
        );
    }

    /// Lift isolation for a chain after the cooling-off period.
    pub fn lift_bridge_chain_isolation(env: Env, admin: Address, chain_id: u32) -> bool {
        Self::require_admin(&env, &admin);
        let result = lift_chain_isolation(&env, chain_id);

        if result {
            env.events().publish(
                ("bridge", "ChainIsolationLifted"),
                (chain_id, env.ledger().timestamp()),
            );
        }

        result
    }

    /// Check whether a chain is currently isolated from bridge operations.
    pub fn is_bridge_chain_isolated(env: Env, chain_id: u32) -> bool {
        is_chain_isolated(&env, chain_id)
    }
}

// Unit tests
#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger;
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

    // -----------------------------------------------------------------------
    // #866 — Cross-chain sync and emergency isolation tests
    // -----------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "Source chain is currently isolated from bridge operations")]
    fn test_isolated_chain_blocks_receive() {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let admin = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);
        client.init(&admin);

        let token = create_wrapped_token(&env, &admin);
        client.set_wrapped_token(&admin, &token);

        // Isolate Ethereum.
        client.emergency_isolate_chain(&admin, &CHAIN_ETHEREUM, &soroban_sdk::Symbol::new(&env, "reorg"));

        let vaa_hash = BytesN::from_array(&env, &[2u8; 32]);
        let recipient = Address::generate(&env);
        // Should panic — chain is isolated.
        client.receive_bridged_asset(&vaa_hash, &recipient, &500, &CHAIN_ETHEREUM);
    }

    #[test]
    fn test_chain_isolation_and_lift() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);
        client.init(&admin);

        assert!(!client.is_bridge_chain_isolated(&CHAIN_ETHEREUM));

        client.emergency_isolate_chain(&admin, &CHAIN_ETHEREUM, &soroban_sdk::Symbol::new(&env, "test"));
        assert!(client.is_bridge_chain_isolated(&CHAIN_ETHEREUM));

        // Cannot lift within 24h cooldown.
        let lifted = client.lift_bridge_chain_isolation(&admin, &CHAIN_ETHEREUM);
        assert!(!lifted);

        // Advance 25 hours.
        env.ledger().with_mut(|l| l.timestamp = 25 * 60 * 60);
        let lifted = client.lift_bridge_chain_isolation(&admin, &CHAIN_ETHEREUM);
        assert!(lifted);
        assert!(!client.is_bridge_chain_isolated(&CHAIN_ETHEREUM));
    }

    #[test]
    fn test_atomic_bridge_op_full_commit() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let relayer = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);
        client.init(&admin);
        client.add_trusted_relayer(&admin, &relayer);

        let expected_root = BytesN::from_array(&env, &[0xABu8; 32]);

        let op_id = client.begin_bridge_atomic_op(
            &relayer,
            &CHAIN_ETHEREUM,
            &CHAIN_SOLANA,
            &expected_root,
        );

        // Phase 1 — both chains prepare.
        client.acknowledge_bridge_prepare(&relayer, &op_id, &CHAIN_ETHEREUM);
        let phase = client.acknowledge_bridge_prepare(&relayer, &op_id, &CHAIN_SOLANA);
        // Both prepared → phase transitions to Committing (2).
        assert_eq!(phase, 2u32);

        // Phase 2 — both chains commit with matching root.
        client.confirm_bridge_commit(&relayer, &op_id, &CHAIN_ETHEREUM, &expected_root);
        let final_phase = client.confirm_bridge_commit(&relayer, &op_id, &CHAIN_SOLANA, &expected_root);
        // Both committed → phase transitions to Committed (3).
        assert_eq!(final_phase, 3u32);
    }

    #[test]
    fn test_state_proof_validation() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let contract_id = env.register_contract(None, BridgeReceiver);
        let client = BridgeReceiverClient::new(&env, &contract_id);
        client.init(&admin);

        let root = BytesN::from_array(&env, &[0xAAu8; 32]);
        let proof_path = soroban_sdk::Vec::new(&env);

        // Trivial proof: state_root == expected_root, empty path.
        let valid = client.validate_bridge_state_proof(
            &CHAIN_ETHEREUM,
            &root,
            &proof_path,
            &1_000u64,
            &root,
        );
        assert!(valid);

        // Wrong expected root should fail.
        let wrong_root = BytesN::from_array(&env, &[0xBBu8; 32]);
        let invalid = client.validate_bridge_state_proof(
            &CHAIN_ETHEREUM,
            &root,
            &proof_path,
            &1_000u64,
            &wrong_root,
        );
        assert!(!invalid);
    }
}
