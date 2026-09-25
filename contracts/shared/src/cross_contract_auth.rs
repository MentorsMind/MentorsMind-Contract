use soroban_sdk::{Address, Env, Symbol};

/// Registry capable of attesting that a given address is the legitimate,
/// currently-authorized instance of a named system interface (e.g.
/// `"staking_v1"`, `"escrow_v1"`).
///
/// Implemented against the `interface_registry` contract's `verify` entry
/// point via dynamic invocation so that `shared` (a dependency of nearly
/// every contract) does not need a hard crate dependency on any specific
/// registry implementation.
pub trait ContractRegistry {
    fn is_authorized(env: &Env, registry: &Address, candidate: &Address, interface_id: Symbol) -> bool;
}

pub struct InterfaceRegistryLookup;

impl ContractRegistry for InterfaceRegistryLookup {
    fn is_authorized(env: &Env, registry: &Address, candidate: &Address, interface_id: Symbol) -> bool {
        env.invoke_contract(
            registry,
            &Symbol::new(env, "verify"),
            soroban_sdk::vec![env, candidate.clone().to_val(), interface_id.to_val()],
        )
    }
}

/// Cross-contract call authentication: confirms that a peer contract address
/// supplied by an admin (or received as a call argument) is both a real
/// account/contract that authorized the current invocation and a contract
/// registered under the expected interface, before it is trusted with
/// privileged state (e.g. wired in as the staking/reputation/insurance
/// integration for an escrow or treasury contract).
///
/// Every check — pass or fail — is published as an event so unauthorized
/// cross-contract call attempts can be reviewed off-chain.
pub struct CrossContractAuth;

impl CrossContractAuth {
    /// Panics if `candidate` is not registered under `interface_id` in the
    /// interface registry at `registry`. Use when wiring a peer contract
    /// address into privileged storage (e.g. `set_staking_contract`).
    pub fn require_authorized_contract(
        env: &Env,
        registry: &Address,
        candidate: &Address,
        interface_id: Symbol,
    ) {
        let authorized =
            InterfaceRegistryLookup::is_authorized(env, registry, candidate, interface_id.clone());

        env.events().publish(
            (Symbol::new(env, "cross_contract_auth"), interface_id),
            (candidate.clone(), authorized),
        );

        if !authorized {
            panic!("cross-contract caller not authorized");
        }
    }
}
