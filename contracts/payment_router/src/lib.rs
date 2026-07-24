#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, BytesN, Env, IntoVal,
    Symbol, Vec,
};

// Source chain constants
pub const CHAIN_STELLAR: u32 = 0;
pub const CHAIN_ETHEREUM: u32 = 2;
pub const CHAIN_SOLANA: u32 = 1;
pub const CHAIN_BSC: u32 = 4;

/// Maximum price deviation (bps) allowed between the oracle TWAP and the
/// spot price before a cross-chain payment is rejected.
/// 500 bps = 5 %.  Bridged payments carry higher manipulation risk, so we
/// use a tighter threshold than the oracle's own circuit breaker.
pub const ORACLE_PRICE_DEVIATION_BPS: i128 = 500;

#[derive(Clone)]
#[contracttype]
pub struct RouterConfig {
    pub admin: Address,
    pub escrow_contract: Address,
    pub bridge_receiver: Address,
    /// Optional oracle contract address.  When set, cross-chain payments are
    /// validated against the oracle's TWAP before being routed.
    pub oracle_contract: Option<Address>,
}

#[derive(Clone)]
#[contracttype]
pub struct PaymentRoute {
    pub escrow_id: u64,
    pub source_chain: u32,
    pub source_tx_hash: BytesN<32>,
    pub learner: Address,
    pub mentor: Address,
    pub amount: i128,
    pub token: Address,
    pub created_at: u64,
}

#[contracttype]
pub struct PaymentRoutedEvent {
    pub source_chain: u32,
    pub source_tx_hash: BytesN<32>,
    pub escrow_id: u64,
    pub learner: Address,
    pub mentor: Address,
    pub amount: i128,
    pub token: Address,
}

/// Event data emitted when a token is approved or rejected in the router.
#[contracttype]
pub struct RouterTokenApprovalEvent {
    pub token: Address,
    pub approved: bool,
}

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Config,
    Route(BytesN<32>),
    ProcessedTx(BytesN<32>),
    EscrowIdCounter,
    /// Token whitelist: DataKey::ApprovedToken(token_address) → bool
    ApprovedToken(Address),
    FeeBps,
    Treasury,
    Timelock,
    Multisig,
    SupportedChains,
}

// TTL constants (in ledgers; ~5 s/ledger → 1 000 000 ≈ 57 days)
const ROUTE_TTL_THRESHOLD: u32 = 500_000;
const ROUTE_TTL_BUMP: u32 = 1_000_000;

#[contract]
pub struct PaymentRouter;

#[contractimpl]
impl PaymentRouter {
    /// Initialize the payment router contract
    pub fn init(env: Env, admin: Address, escrow_contract: Address, bridge_receiver: Address) {
        // Check if already initialized
        if env.storage().instance().has(&DataKey::Config) {
            panic!("Already initialized");
        }

        let mut supported_chains = Vec::new(&env);
        supported_chains.push_back(CHAIN_STELLAR);
        supported_chains.push_back(CHAIN_ETHEREUM);
        supported_chains.push_back(CHAIN_SOLANA);
        supported_chains.push_back(CHAIN_BSC);

        let config = RouterConfig {
            admin: admin.clone(),
            escrow_contract,
            bridge_receiver,
            oracle_contract: None,
        };

        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::SupportedChains, &supported_chains);
        env.storage()
            .instance()
            .set(&DataKey::EscrowIdCounter, &0u64);

        // Emit initialization event
        env.events()
            .publish((symbol_short!("router"), symbol_short!("init")), admin);
    }

    // -----------------------------------------------------------------------
    // Token Whitelist Management (admin only)
    // -----------------------------------------------------------------------

    /// Add or remove an approved token from the router whitelist (admin only).
    /// Only whitelisted tokens can be routed through the payment router.
    pub fn set_approved_token(env: Env, token_address: Address, approved: bool) {
        let config = Self::get_config(env.clone());
        config.admin.require_auth();

        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage().persistent().set(&key, &approved);

        // Emit token approval/rejection event
        if approved {
            env.events().publish(
                (symbol_short!("router"), symbol_short!("tok_appr")),
                RouterTokenApprovalEvent {
                    token: token_address,
                    approved: true,
                },
            );
        } else {
            env.events().publish(
                (symbol_short!("router"), symbol_short!("tok_rej")),
                RouterTokenApprovalEvent {
                    token: token_address,
                    approved: false,
                },
            );
        }
    }

    /// Check if a token is on the router's approved whitelist.
    pub fn is_token_approved(env: Env, token_address: Address) -> bool {
        Self::_is_token_approved(&env, &token_address)
    }

    /// Internal token whitelist check.
    fn _is_token_approved(env: &Env, token_address: &Address) -> bool {
        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage()
            .persistent()
            .get::<_, bool>(&key)
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Payment Routing
    // -----------------------------------------------------------------------

    /// Route a payment from any supported chain to create an escrow
    ///
    /// # Arguments
    /// * `source_chain` - The chain ID where payment originated (0 for Stellar native)
    /// * `source_tx_hash` - The transaction hash on the source chain
    /// * `learner` - The learner's address
    /// * `mentor` - The mentor's address  
    /// * `amount` - The payment amount
    /// * `token` - The token contract address (must be whitelisted)
    ///
    /// # Returns
    /// * The escrow ID created
    pub fn route_payment(
        env: Env,
        source_chain: u32,
        source_tx_hash: BytesN<32>,
        learner: Address,
        mentor: Address,
        amount: i128,
        token: Address,
    ) -> u64 {
        // *** STRICT TOKEN WHITELIST VALIDATION ***
        // The token MUST be on the router's approved list before routing.
        // This prevents malicious token contracts from being used to
        // circumvent restrictions in the escrow contract.
        if !Self::_is_token_approved(&env, &token) {
            panic!("Token not approved for routing");
        }

        // Verify the source transaction
        Self::verify_source_transaction(
            &env,
            source_chain,
            &source_tx_hash,
            &learner,
            amount,
            &token,
        );

        // Check for duplicate routing
        let processed_key = DataKey::ProcessedTx(source_tx_hash.clone());
        if env.storage().persistent().has(&processed_key) {
            panic!("Transaction already routed");
        }

        // Verify amount is positive
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        // Verify learner authorization for direct Stellar payments
        // For bridged payments, verification happens via bridge receiver
        if source_chain == CHAIN_STELLAR {
            learner.require_auth();
        }

        // Get config
        let config = Self::get_config(env.clone());

        // Oracle price-manipulation check for cross-chain payments.
        if let Some(ref oracle) = config.oracle_contract {
            Self::validate_oracle_price(&env, oracle, &token);
        }

        // Calculate fee and net amount
        let fee = Self::calculate_fee(env.clone(), amount);
        let net_amount = amount.checked_sub(fee).unwrap_or(0);

        // For Stellar direct payments, transfer tokens from learner to escrow
        if source_chain == CHAIN_STELLAR {
            let token_client = token::Client::new(&env, &token);

            // Verify learner has sufficient balance
            if token_client.balance(&learner) < amount {
                panic!("Insufficient token balance");
            }

            // Transfer the fee to the treasury if applicable
            if fee > 0 {
                let treasury: Address = env.storage().instance().get(&DataKey::Treasury).expect("Treasury not set");
                token_client.transfer(&learner, &treasury, &fee);
            }
            
            // Note: The escrow contract will ALSO try to transfer the net_amount from the learner
            // depending on how create_escrow is structured. If router transfers to escrow, 
            // escrow might transfer again. We assume escrow expects the learner to pay directly.
        }

        // Generate a unique session ID for the escrow
        let session_id = Self::generate_session_id(&env, &source_tx_hash, source_chain);

        // Create escrow via cross-contract call using net_amount
        let escrow_id = Self::create_escrow(
            &env,
            &config.escrow_contract,
            mentor.clone(),
            learner.clone(),
            net_amount,
            session_id,
            token.clone(),
        );

        // Store the route mapping
        let route = PaymentRoute {
            escrow_id,
            source_chain,
            source_tx_hash: source_tx_hash.clone(),
            learner: learner.clone(),
            mentor: mentor.clone(),
            amount: net_amount, // store the routed amount
            token: token.clone(),
            created_at: env.ledger().timestamp(),
        };

        let route_key = DataKey::Route(source_tx_hash.clone());
        env.storage().persistent().set(&route_key, &route);
        env.storage().persistent().extend_ttl(&route_key, ROUTE_TTL_THRESHOLD, ROUTE_TTL_BUMP);
        env.storage().persistent().set(&processed_key, &true);
        env.storage().persistent().extend_ttl(&processed_key, ROUTE_TTL_THRESHOLD, ROUTE_TTL_BUMP);

        // Update counter
        let counter: u64 = env
            .storage()
            .instance()
            .get(&DataKey::EscrowIdCounter)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::EscrowIdCounter, &(counter.checked_add(1).expect("Overflow")));

        // Emit payment routed event
        let event = PaymentRoutedEvent {
            source_chain,
            source_tx_hash: source_tx_hash.clone(),
            escrow_id,
            learner: learner.clone(),
            mentor: mentor.clone(),
            amount: net_amount,
            token: token.clone(),
        };
        Self::emit_payment_routed(&env, event);

        escrow_id
    }

    /// Calculate routing fee for a given amount
    pub fn calculate_fee(env: Env, amount: i128) -> i128 {
        let fee_bps: u32 = env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0);
        if fee_bps > 0 {
            amount.checked_mul(fee_bps as i128).unwrap_or(0) / 10_000
        } else {
            0
        }
    }

    /// Update routing fee bps (admin only)
    pub fn set_fee_bps(env: Env, fee_bps: u32) {
        let config = Self::get_config(env.clone());
        config.admin.require_auth();
        if fee_bps > 10_000 {
            panic!("Fee > 10000 bps");
        }
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
    }

    /// Update routing treasury (admin only)
    pub fn set_treasury(env: Env, treasury: Address) {
        let config = Self::get_config(env.clone());
        config.admin.require_auth();
        env.storage().instance().set(&DataKey::Treasury, &treasury);
    }

    /// Get current routing fee bps
    pub fn get_fee_bps(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0)
    }

    /// Get current treasury
    pub fn get_treasury(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Treasury).expect("Treasury not set")
    }

    /// Get the escrow ID for a given source transaction hash
    pub fn get_route(env: Env, source_tx_hash: BytesN<32>) -> u64 {
        let route_key = DataKey::Route(source_tx_hash);
        env.storage()
            .persistent()
            .extend_ttl(&route_key, ROUTE_TTL_THRESHOLD, ROUTE_TTL_BUMP);
        let route: PaymentRoute = env
            .storage()
            .persistent()
            .get(&route_key)
            .expect("Route not found");
        route.escrow_id
    }

    /// Get full route details for a source transaction hash
    pub fn get_route_details(env: Env, source_tx_hash: BytesN<32>) -> PaymentRoute {
        let route_key = DataKey::Route(source_tx_hash);
        env.storage()
            .persistent()
            .extend_ttl(&route_key, ROUTE_TTL_THRESHOLD, ROUTE_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&route_key)
            .expect("Route not found")
    }

    /// Check if a transaction has already been routed
    pub fn is_tx_processed(env: Env, source_tx_hash: BytesN<32>) -> bool {
        let processed_key = DataKey::ProcessedTx(source_tx_hash);
        let has = env.storage().persistent().has(&processed_key);
        if has {
            env.storage().persistent().extend_ttl(&processed_key, ROUTE_TTL_THRESHOLD, ROUTE_TTL_BUMP);
        }
        has
    }

    /// Get the list of supported chains
    pub fn get_supported_chains(env: Env) -> Vec<u32> {
        env.storage()
            .instance()
            .get(&DataKey::SupportedChains)
            .unwrap_or(Vec::new(&env))
    }

    /// Add a supported chain (admin only)
    pub fn add_supported_chain(env: Env, chain_id: u32) {
        let config = Self::get_config(env.clone());
        config.admin.require_auth();

        let mut supported_chains: Vec<u32> = env
            .storage()
            .instance()
            .get(&DataKey::SupportedChains)
            .unwrap_or(Vec::new(&env));

        // Check if chain already exists
        let exists = supported_chains.iter().any(|c| c == chain_id);
        if exists {
            panic!("Chain already supported");
        }

        supported_chains.push_back(chain_id);
        env.storage().instance().set(&DataKey::SupportedChains, &supported_chains);
    }

    /// Remove a supported chain (admin only)
    pub fn remove_supported_chain(env: Env, chain_id: u32) {
        let config = Self::get_config(env.clone());
        config.admin.require_auth();

        // Cannot remove Stellar native chain
        if chain_id == CHAIN_STELLAR {
            panic!("Cannot remove Stellar native chain");
        }

        let supported_chains: Vec<u32> = env
            .storage()
            .instance()
            .get(&DataKey::SupportedChains)
            .unwrap_or(Vec::new(&env));

        let mut new_chains = Vec::new(&env);
        for chain in supported_chains.iter() {
            if chain != chain_id {
                new_chains.push_back(chain);
            }
        }

        env.storage().instance().set(&DataKey::SupportedChains, &new_chains);
    }

    /// Update timelock contract address (admin only)
    pub fn set_timelock(env: Env, timelock: Address) {
        let config = Self::get_config(env.clone());
        config.admin.require_auth();
        env.storage().instance().set(&DataKey::Timelock, &timelock);
    }

    /// Update multisig contract address (admin only)
    pub fn set_multisig(env: Env, multisig: Address) {
        let config = Self::get_config(env.clone());
        config.admin.require_auth();
        env.storage().instance().set(&DataKey::Multisig, &multisig);
    }

    /// Schedule an update to the escrow contract
    pub fn schedule_escrow_contract(env: Env, new_escrow: Address) {
        let multisig: Address = env.storage().instance().get(&DataKey::Multisig).expect("Multisig not set");
        multisig.require_auth();

        let timelock: Address = env.storage().instance().get(&DataKey::Timelock).expect("Timelock not set");

        let mut args = Vec::new(&env);
        args.push_back(new_escrow.into_val(&env));

        env.invoke_contract::<BytesN<32>>(
            &timelock,
            &Symbol::new(&env, "schedule"),
            (
                env.current_contract_address(), // proposer
                env.current_contract_address(), // target
                Symbol::new(&env, "set_escrow_contract"), // function
                args,
                48u64 * 60 * 60, // 48 hours delay
            ).into_val(&env)
        );
    }
    
    /// Schedule an update to the bridge receiver
    pub fn schedule_bridge_receiver(env: Env, new_bridge: Address) {
        let multisig: Address = env.storage().instance().get(&DataKey::Multisig).expect("Multisig not set");
        multisig.require_auth();

        let timelock: Address = env.storage().instance().get(&DataKey::Timelock).expect("Timelock not set");

        let mut args = Vec::new(&env);
        args.push_back(new_bridge.into_val(&env));

        env.invoke_contract::<BytesN<32>>(
            &timelock,
            &Symbol::new(&env, "schedule"),
            (
                env.current_contract_address(),
                env.current_contract_address(),
                Symbol::new(&env, "set_bridge_receiver"),
                args,
                48u64 * 60 * 60,
            ).into_val(&env)
        );
    }

    /// Update escrow contract address (timelock only)
    pub fn set_escrow_contract(env: Env, escrow_contract: Address) {
        let timelock: Address = env.storage().instance().get(&DataKey::Timelock).expect("Timelock not set");
        timelock.require_auth();

        let config = Self::get_config(env.clone());
        let old_escrow = config.escrow_contract.clone();

        let mut new_config = config;
        new_config.escrow_contract = escrow_contract.clone();
        env.storage().instance().set(&DataKey::Config, &new_config);

        env.events().publish(
            (symbol_short!("router"), symbol_short!("escr_set")),
            (old_escrow, escrow_contract)
        );
    }

    /// Update bridge receiver address (timelock only)
    pub fn set_bridge_receiver(env: Env, bridge_receiver: Address) {
        let timelock: Address = env.storage().instance().get(&DataKey::Timelock).expect("Timelock not set");
        timelock.require_auth();

        let config = Self::get_config(env.clone());
        let old_bridge = config.bridge_receiver.clone();

        let mut new_config = config;
        new_config.bridge_receiver = bridge_receiver.clone();
        env.storage().instance().set(&DataKey::Config, &new_config);

        env.events().publish(
            (symbol_short!("router"), symbol_short!("brdg_set")),
            (old_bridge, bridge_receiver)
        );
    }

    /// Set or update the oracle contract address (admin only).
    pub fn set_oracle(env: Env, oracle_contract: Address) {
        let config = Self::get_config(env.clone());
        config.admin.require_auth();

        let mut new_config = config;
        new_config.oracle_contract = Some(oracle_contract.clone());
        env.storage().instance().set(&DataKey::Config, &new_config);

        env.events().publish(
            (symbol_short!("router"), symbol_short!("orc_set")),
            oracle_contract,
        );
    }

    /// Remove the oracle integration (admin only).
    pub fn clear_oracle(env: Env) {
        let config = Self::get_config(env.clone());
        config.admin.require_auth();

        let mut new_config = config;
        new_config.oracle_contract = None;
        env.storage().instance().set(&DataKey::Config, &new_config);
    }

    /// Return the configured oracle contract address, if any.
    pub fn get_oracle(env: Env) -> Option<Address> {
        Self::get_config(env).oracle_contract
    }

    /// Get the router configuration
    pub fn get_config(env: Env) -> RouterConfig {
        env.storage()
            .instance()
            .get(&DataKey::Config)
            .expect("Router not initialized")
    }

    /// Get total number of routed payments
    pub fn get_route_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::EscrowIdCounter)
            .unwrap_or(0)
    }

    // Helper functions

    fn verify_source_transaction(
        env: &Env,
        source_chain: u32,
        source_tx_hash: &BytesN<32>,
        _learner: &Address,
        _amount: i128,
        _token: &Address,
    ) {
        let config = Self::get_config(env.clone());
        let supported_chains: Vec<u32> = env
            .storage()
            .instance()
            .get(&DataKey::SupportedChains)
            .unwrap_or(Vec::new(env));

        // Check if source chain is supported
        let is_supported = supported_chains
            .iter()
            .any(|chain| chain == source_chain);
        if !is_supported {
            panic!("Source chain not supported");
        }

        // For bridged transactions, verify via bridge receiver
        if source_chain != CHAIN_STELLAR {
            // Check if the bridge receiver has processed this VAA
            let is_processed: bool = env.invoke_contract(
                &config.bridge_receiver,
                &Symbol::new(env, "is_vaa_processed"),
                (source_tx_hash.clone(),).into_val(env),
            );

            if !is_processed {
                panic!("Bridge transaction not verified");
            }
        }
    }

    /// Validate that the token price reported by the oracle has not been
    /// manipulated.
    fn validate_oracle_price(env: &Env, oracle: &Address, token: &Address) {
        let maybe_asset: Option<Symbol> = env.invoke_contract(
            oracle,
            &Symbol::new(env, "get_asset_for_token"),
            (token.clone(),).into_val(env),
        );

        let asset = match maybe_asset {
            Some(a) => a,
            None => return, // Token not tracked by oracle — skip check.
        };

        let manipulated: bool = env.invoke_contract(
            oracle,
            &Symbol::new(env, "is_price_manipulated"),
            (asset, ORACLE_PRICE_DEVIATION_BPS).into_val(env),
        );

        if manipulated {
            panic!("oracle: price manipulation detected — payment routing blocked");
        }
    }

    fn create_escrow(
        env: &Env,
        escrow_contract: &Address,
        mentor: Address,
        learner: Address,
        amount: i128,
        session_id: Symbol,
        token: Address,
    ) -> u64 {
        // Use a default session end time (30 days from now)
        let session_end_time = env.ledger().timestamp() + (30 * 24 * 60 * 60);
        let total_sessions = 1u32;

        // Call create_escrow on the escrow contract with individual parameters
        let escrow_id: u64 = env.invoke_contract(
            escrow_contract,
            &Symbol::new(env, "create_escrow"),
            (
                mentor,
                learner,
                amount,
                session_id,
                token,
                session_end_time,
                total_sessions,
            )
                .into_val(env),
        );

        escrow_id
    }

    fn generate_session_id(env: &Env, _source_tx_hash: &BytesN<32>, _source_chain: u32) -> Symbol {
        let counter: u64 = env
            .storage()
            .instance()
            .get(&DataKey::EscrowIdCounter)
            .unwrap_or(0);
        
        // Create a symbol with "RT_" prefix followed by the counter
        let symbol_str = format!("RT_{}", counter);
        Symbol::new(env, &symbol_str)
    }

    fn emit_payment_routed(env: &Env, event: PaymentRoutedEvent) {
        env.events()
            .publish((symbol_short!("router"), symbol_short!("routed")), event);
    }
}

// Unit tests
#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger;

    // =========================================================================
    // Mock Bridge Receiver Contract
    // =========================================================================

    #[contract]
    pub struct MockBridgeReceiver;

    #[contracttype]
    #[derive(Clone)]
    pub enum MockBridgeKey {
        ProcessedVAA(BytesN<32>),
    }

    #[contractimpl]
    impl MockBridgeReceiver {
        pub fn set_vaa_processed(env: Env, vaa_hash: BytesN<32>) {
            env.storage()
                .instance()
                .set(&MockBridgeKey::ProcessedVAA(vaa_hash), &true);
        }

        pub fn is_vaa_processed(env: Env, vaa_hash: BytesN<32>) -> bool {
            env.storage()
                .instance()
                .has(&MockBridgeKey::ProcessedVAA(vaa_hash))
        }
    }

    // =========================================================================
    // Mock Escrow Contract
    // =========================================================================

    #[contract]
    pub struct MockEscrow;

    #[contracttype]
    #[derive(Clone)]
    pub enum MockEscrowKey {
        EscrowCount,
        Escrow(u64),
        Session(Symbol),
    }

    #[contractimpl]
    impl MockEscrow {
        pub fn create_escrow(
            env: Env,
            _mentor: Address,
            _learner: Address,
            _amount: i128,
            _session_id: Symbol,
            _token_address: Address,
            _session_end_time: u64,
            _total_sessions: u32,
        ) -> u64 {
            let mut count: u64 = env
                .storage()
                .instance()
                .get(&MockEscrowKey::EscrowCount)
                .unwrap_or(0);
            count += 1;
            env.storage()
                .instance()
                .set(&MockEscrowKey::EscrowCount, &count);
            count
        }

        pub fn get_escrow_count(env: Env) -> u64 {
            env.storage()
                .instance()
                .get(&MockEscrowKey::EscrowCount)
                .unwrap_or(0)
        }
    }

    // =========================================================================
    // Mock Token Contract
    // =========================================================================

    #[contract]
    pub struct MockToken;

    #[contracttype]
    #[derive(Clone)]
    pub enum MockTokenKey {
        Balance(Address),
    }

    #[contractimpl]
    impl MockToken {
        pub fn mint(env: Env, to: Address, amount: i128) {
            let bal: i128 = env
                .storage()
                .instance()
                .get(&MockTokenKey::Balance(to.clone()))
                .unwrap_or(0);
            env.storage()
                .instance()
                .set(&MockTokenKey::Balance(to), &(bal + amount));
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .instance()
                .get(&MockTokenKey::Balance(id))
                .unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            let from_bal = Self::balance(env.clone(), from.clone());
            assert!(from_bal >= amount, "Insufficient balance");
            let to_bal = Self::balance(env.clone(), to.clone());
            env.storage()
                .instance()
                .set(&MockTokenKey::Balance(from), &(from_bal - amount));
            env.storage()
                .instance()
                .set(&MockTokenKey::Balance(to), &(to_bal + amount));
        }

        pub fn spendable_balance(env: Env, id: Address) -> i128 {
            Self::balance(env, id)
        }
    }

    // =========================================================================
    // Test Setup
    // =========================================================================

    fn setup_env(env: &Env) -> (Address, Address, Address, Address, PaymentRouterClient<'_>) {
        let admin = Address::generate(env);
        let escrow_contract = Address::generate(env);
        let bridge_receiver = Address::generate(env);
        let token = Address::generate(env);

        let contract_id = env.register_contract(None, PaymentRouter);
        let client = PaymentRouterClient::new(env, &contract_id);

        (admin, escrow_contract, bridge_receiver, token, client)
    }

    /// Setup with mock contracts for integration testing
    struct IntegrationFixture {
        env: Env,
        router_client: PaymentRouterClient<'static>,
        bridge_client: MockBridgeReceiverClient<'static>,
        escrow_client: MockEscrowClient<'static>,
        token_client: MockTokenClient<'static>,
        admin: Address,
    }

    impl IntegrationFixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);

            // Register mock contracts
            let bridge_id = env.register_contract(None, MockBridgeReceiver);
            let escrow_id = env.register_contract(None, MockEscrow);
            let token_id = env.register_contract(None, MockToken);

            // Register payment router
            let router_id = env.register_contract(None, PaymentRouter);

            // Initialize router with mock contract addresses
            let router_client = PaymentRouterClient::new(&env, &router_id);
            router_client.init(&admin, &escrow_id, &bridge_id);

            // *** Approve the mock token in the router's whitelist ***
            router_client.set_approved_token(&token_id, &true);

            let fixture = IntegrationFixture {
                env,
                router_client,
                bridge_client: MockBridgeReceiverClient::new(&env, &bridge_id),
                escrow_client: MockEscrowClient::new(&env, &escrow_id),
                token_client: MockTokenClient::new(&env, &token_id),
                admin,
            };

            fixture
        }

        fn fund_learner(&self, learner: &Address, amount: i128) {
            self.token_client.mint(learner, &amount);
        }

        fn mark_bridge_vaa_processed(&self, vaa_hash: &BytesN<32>) {
            self.bridge_client.set_vaa_processed(vaa_hash);
        }
    }

    // =========================================================================
    // Basic tests
    // =========================================================================

    #[test]
    fn test_init() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);

        client.init(&admin, &escrow_contract, &bridge_receiver);

        let config = client.get_config();
        assert_eq!(config.admin, admin);
        assert_eq!(config.escrow_contract, escrow_contract);
        assert_eq!(config.bridge_receiver, bridge_receiver);

        let chains = client.get_supported_chains();
        assert_eq!(chains.len(), 4);
        assert_eq!(chains.get(0).unwrap(), CHAIN_STELLAR);
        assert_eq!(chains.get(1).unwrap(), CHAIN_ETHEREUM);
    }

    #[test]
    #[should_panic(expected = "Already initialized")]
    fn test_double_init() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);

        client.init(&admin, &escrow_contract, &bridge_receiver);
        client.init(&admin, &escrow_contract, &bridge_receiver);
    }

    #[test]
    fn test_add_supported_chain() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);
        env.mock_all_auths();

        client.init(&admin, &escrow_contract, &bridge_receiver);

        // Add a new chain (e.g., Arbitrum = 23)
        client.add_supported_chain(&23);

        let chains = client.get_supported_chains();
        assert_eq!(chains.len(), 5);
    }

    #[test]
    #[should_panic(expected = "Chain already supported")]
    fn test_add_duplicate_chain() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);
        env.mock_all_auths();

        client.init(&admin, &escrow_contract, &bridge_receiver);
        client.add_supported_chain(&CHAIN_ETHEREUM);
    }

    #[test]
    fn test_remove_supported_chain() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);
        env.mock_all_auths();

        client.init(&admin, &escrow_contract, &bridge_receiver);
        client.remove_supported_chain(&CHAIN_BSC);

        let chains = client.get_supported_chains();
        assert_eq!(chains.len(), 3);

        let contains_bsc = chains.iter().any(|c| c == CHAIN_BSC);
        assert!(!contains_bsc);
    }

    #[test]
    #[should_panic(expected = "Cannot remove Stellar native chain")]
    fn test_remove_stellar_chain() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);
        env.mock_all_auths();

        client.init(&admin, &escrow_contract, &bridge_receiver);
        client.remove_supported_chain(&CHAIN_STELLAR);
    }

    #[test]
    fn test_is_tx_processed_not_found() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);

        client.init(&admin, &escrow_contract, &bridge_receiver);

        let tx_hash = BytesN::from_array(&env, &[0u8; 32]);
        assert!(!client.is_tx_processed(&tx_hash));
    }

    #[test]
    #[should_panic(expected = "Source chain not supported")]
    fn test_route_payment_unsupported_chain() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, token, client) = setup_env(&env);
        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);
        env.mock_all_auths();

        client.init(&admin, &escrow_contract, &bridge_receiver);
        // Approve the token so it's not rejected for that reason
        client.set_approved_token(&token, &true);

        let tx_hash = BytesN::from_array(&env, &[1u8; 32]);

        // Try to route from unsupported chain (99)
        client.route_payment(&99, &tx_hash, &learner, &mentor, &1000, &token);
    }

    #[test]
    #[should_panic(expected = "Amount must be positive")]
    fn test_route_payment_zero_amount() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, token, client) = setup_env(&env);
        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);
        env.mock_all_auths();

        client.init(&admin, &escrow_contract, &bridge_receiver);
        client.set_approved_token(&token, &true);

        let tx_hash = BytesN::from_array(&env, &[1u8; 32]);

        client.route_payment(&CHAIN_STELLAR, &tx_hash, &learner, &mentor, &0, &token);
    }

    #[test]
    fn test_set_escrow_contract() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);
        let new_escrow = Address::generate(&env);
        let timelock = Address::generate(&env);
        env.mock_all_auths();

        client.init(&admin, &escrow_contract, &bridge_receiver);
        client.set_timelock(&timelock);
        client.set_escrow_contract(&new_escrow);

        let config = client.get_config();
        assert_eq!(config.escrow_contract, new_escrow);
    }

    #[test]
    fn test_set_bridge_receiver() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);
        let new_bridge = Address::generate(&env);
        let timelock = Address::generate(&env);
        env.mock_all_auths();

        client.init(&admin, &escrow_contract, &bridge_receiver);
        client.set_timelock(&timelock);
        client.set_bridge_receiver(&new_bridge);

        let config = client.get_config();
        assert_eq!(config.bridge_receiver, new_bridge);
    }

    #[test]
    fn test_get_route_count_initial() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);

        client.init(&admin, &escrow_contract, &bridge_receiver);

        let count = client.get_route_count();
        assert_eq!(count, 0);
    }

    // =========================================================================
    // Token Whitelist Tests
    // =========================================================================

    #[test]
    fn test_token_whitelist_toggle() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);
        env.mock_all_auths();
        client.init(&admin, &escrow_contract, &bridge_receiver);

        let token = Address::generate(&env);
        assert!(!client.is_token_approved(&token));
        client.set_approved_token(&token, &true);
        assert!(client.is_token_approved(&token));
        client.set_approved_token(&token, &false);
        assert!(!client.is_token_approved(&token));
    }

    /// Test: Unapproved token is rejected by route_payment
    #[test]
    #[should_panic(expected = "Token not approved for routing")]
    fn test_route_payment_unapproved_token() {
        let fixture = IntegrationFixture::setup();
        let learner = Address::generate(&fixture.env);
        let mentor = Address::generate(&fixture.env);

        // Create a token that is NOT approved
        let malicious_token = Address::generate(&fixture.env);

        let tx_hash = BytesN::from_array(&fixture.env, &[1u8; 32]);
        fixture.fund_learner(&learner, 1000);

        // This should panic because malicious_token is not whitelisted
        fixture.router_client.route_payment(
            &CHAIN_STELLAR,
            &tx_hash,
            &learner,
            &mentor,
            &1000,
            &malicious_token,
        );
    }

    /// Test: Revoked token is rejected by route_payment
    #[test]
    #[should_panic(expected = "Token not approved for routing")]
    fn test_route_payment_revoked_token() {
        let fixture = IntegrationFixture::setup();
        let learner = Address::generate(&fixture.env);
        let mentor = Address::generate(&fixture.env);

        // Revoke the previously approved token
        fixture.router_client.set_approved_token(&fixture.token_client.address, &false);

        let tx_hash = BytesN::from_array(&fixture.env, &[1u8; 32]);
        fixture.fund_learner(&learner, 1000);

        // This should panic because the token has been revoked
        fixture.router_client.route_payment(
            &CHAIN_STELLAR,
            &tx_hash,
            &learner,
            &mentor,
            &1000,
            &fixture.token_client.address,
        );
    }

    /// Test: Unknown token address defaults to not-approved
    #[test]
    fn test_unknown_token_not_approved() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);
        client.init(&admin, &escrow_contract, &bridge_receiver);

        for _ in 0..5 {
            let random_token = Address::generate(&env);
            assert!(
                !client.is_token_approved(&random_token),
                "unknown tokens must default to not-approved"
            );
        }
    }

    // =========================================================================
    // Integration tests (with token whitelist)
    // =========================================================================

    #[test]
    fn test_stellar_direct_payment() {
        let fixture = IntegrationFixture::setup();
        let learner = Address::generate(&fixture.env);
        let mentor = Address::generate(&fixture.env);

        let amount = 1000i128;
        fixture.fund_learner(&learner, amount);

        let tx_hash = BytesN::from_array(&fixture.env, &[1u8; 32]);

        let escrow_id = fixture.router_client.route_payment(
            &CHAIN_STELLAR,
            &tx_hash,
            &learner,
            &mentor,
            &amount,
            &fixture.token_client.address,
        );

        assert_eq!(escrow_id, 1);
        let stored_escrow_id = fixture.router_client.get_route(&tx_hash);
        assert_eq!(stored_escrow_id, escrow_id);
        assert!(fixture.router_client.is_tx_processed(&tx_hash));
        assert_eq!(fixture.router_client.get_route_count(), 1);

        let route = fixture.router_client.get_route_details(&tx_hash);
        assert_eq!(route.source_chain, CHAIN_STELLAR);
        assert_eq!(route.learner, learner);
        assert_eq!(route.mentor, mentor);
        assert_eq!(route.amount, amount);
    }

    #[test]
    fn test_bridged_eth_payment() {
        let fixture = IntegrationFixture::setup();
        let learner = Address::generate(&fixture.env);
        let mentor = Address::generate(&fixture.env);

        let tx_hash = BytesN::from_array(&fixture.env, &[2u8; 32]);
        let amount = 5000i128;

        fixture.mark_bridge_vaa_processed(&tx_hash);

        let escrow_id = fixture.router_client.route_payment(
            &CHAIN_ETHEREUM,
            &tx_hash,
            &learner,
            &mentor,
            &amount,
            &fixture.token_client.address,
        );

        assert_eq!(escrow_id, 1);
        let route = fixture.router_client.get_route_details(&tx_hash);
        assert_eq!(route.source_chain, CHAIN_ETHEREUM);
        assert_eq!(route.amount, amount);
        assert!(fixture.router_client.is_tx_processed(&tx_hash));
    }

    #[test]
    #[should_panic(expected = "Transaction already routed")]
    fn test_duplicate_routing_prevention() {
        let fixture = IntegrationFixture::setup();
        let learner = Address::generate(&fixture.env);
        let mentor = Address::generate(&fixture.env);

        let amount = 1000i128;
        fixture.fund_learner(&learner, amount);

        let tx_hash = BytesN::from_array(&fixture.env, &[3u8; 32]);

        let escrow_id_1 = fixture.router_client.route_payment(
            &CHAIN_STELLAR,
            &tx_hash,
            &learner,
            &mentor,
            &amount,
            &fixture.token_client.address,
        );
        assert_eq!(escrow_id_1, 1);

        // Attempt duplicate — should panic
        fixture.router_client.route_payment(
            &CHAIN_STELLAR,
            &tx_hash,
            &learner,
            &mentor,
            &amount,
            &fixture.token_client.address,
        );
    }

    #[test]
    fn test_multiple_routes_different_ids() {
        let fixture = IntegrationFixture::setup();
        let learner = Address::generate(&fixture.env);
        let mentor = Address::generate(&fixture.env);

        fixture.fund_learner(&learner, 3000);

        let tx_hash_1 = BytesN::from_array(&fixture.env, &[1u8; 32]);
        let escrow_id_1 = fixture.router_client.route_payment(
            &CHAIN_STELLAR,
            &tx_hash_1,
            &learner,
            &mentor,
            &1000,
            &fixture.token_client.address,
        );

        let tx_hash_2 = BytesN::from_array(&fixture.env, &[2u8; 32]);
        let escrow_id_2 = fixture.router_client.route_payment(
            &CHAIN_STELLAR,
            &tx_hash_2,
            &learner,
            &mentor,
            &2000,
            &fixture.token_client.address,
        );

        assert_ne!(escrow_id_1, escrow_id_2);
        assert_eq!(escrow_id_1, 1);
        assert_eq!(escrow_id_2, 2);
        assert_eq!(fixture.router_client.get_route_count(), 2);
        assert_eq!(fixture.router_client.get_route(&tx_hash_1), escrow_id_1);
        assert_eq!(fixture.router_client.get_route(&tx_hash_2), escrow_id_2);
    }

    #[test]
    fn test_bridged_payments_different_chains() {
        let fixture = IntegrationFixture::setup();
        let learner = Address::generate(&fixture.env);
        let mentor = Address::generate(&fixture.env);

        let tx_hash_eth = BytesN::from_array(&fixture.env, &[10u8; 32]);
        fixture.mark_bridge_vaa_processed(&tx_hash_eth);
        let escrow_id_eth = fixture.router_client.route_payment(
            &CHAIN_ETHEREUM,
            &tx_hash_eth,
            &learner,
            &mentor,
            &3000,
            &fixture.token_client.address,
        );

        let tx_hash_bsc = BytesN::from_array(&fixture.env, &[20u8; 32]);
        fixture.mark_bridge_vaa_processed(&tx_hash_bsc);
        let escrow_id_bsc = fixture.router_client.route_payment(
            &CHAIN_BSC,
            &tx_hash_bsc,
            &learner,
            &mentor,
            &4000,
            &fixture.token_client.address,
        );

        assert_ne!(escrow_id_eth, escrow_id_bsc);

        let route_eth = fixture.router_client.get_route_details(&tx_hash_eth);
        let route_bsc = fixture.router_client.get_route_details(&tx_hash_bsc);

        assert_eq!(route_eth.source_chain, CHAIN_ETHEREUM);
        assert_eq!(route_bsc.source_chain, CHAIN_BSC);
    }

    #[test]
    #[should_panic(expected = "Route not found")]
    fn test_get_route_not_found() {
        let env = Env::default();
        let (admin, escrow_contract, bridge_receiver, _, client) = setup_env(&env);

        client.init(&admin, &escrow_contract, &bridge_receiver);

        let tx_hash = BytesN::from_array(&env, &[99u8; 32]);
        client.get_route(&tx_hash);
    }

    #[test]
    #[should_panic(expected = "Amount must be positive")]
    fn test_route_payment_negative_amount() {
        let fixture = IntegrationFixture::setup();
        let learner = Address::generate(&fixture.env);
        let mentor = Address::generate(&fixture.env);

        let tx_hash = BytesN::from_array(&fixture.env, &[1u8; 32]);
        fixture.router_client.route_payment(
            &CHAIN_STELLAR,
            &tx_hash,
            &learner,
            &mentor,
            &-100,
            &fixture.token_client.address,
        );
    }

    #[test]
    #[should_panic(expected = "Bridge transaction not verified")]
    fn test_bridged_payment_not_verified() {
        let fixture = IntegrationFixture::setup();
        let learner = Address::generate(&fixture.env);
        let mentor = Address::generate(&fixture.env);

        let tx_hash = BytesN::from_array(&fixture.env, &[5u8; 32]);

        fixture.router_client.route_payment(
            &CHAIN_ETHEREUM,
            &tx_hash,
            &learner,
            &mentor,
            &1000,
            &fixture.token_client.address,
        );
    }
}
