#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Vec, Symbol, symbol_short};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionStatus {
    Pending,
    Executed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Transaction {
    pub id: u64,
    pub proposer: Address,
    pub to: Address,
    pub token: Address,
    pub amount: i128,
    pub execute_after: u64,
    pub status: TransactionStatus,
    pub approvals: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Signer(Address),     // Value: bool
    SignerCount,         // Value: u32
    Threshold,           // Value: u32
    TransCount,          // Value: u64
    Transaction(u64),    // Value: Transaction
    Approval(u64, Address), // Value: bool
}

const TTL_THRESHOLD: u32 = 500_000;
const TTL_BUMP: u32 = 1_000_000;

#[contract]
pub struct MultiSigContract;

#[contractimpl]
impl MultiSigContract {
    pub fn initialize(env: Env, admin: Address, signers: Vec<Address>, threshold: u32) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        
        if signers.len() == 0 || threshold == 0 || threshold > signers.len() {
            panic!("Invalid threshold or signers");
        }
        
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().extend_ttl(&DataKey::Admin, TTL_THRESHOLD, TTL_BUMP);
        
        env.storage().persistent().set(&DataKey::Threshold, &threshold);
        env.storage().persistent().extend_ttl(&DataKey::Threshold, TTL_THRESHOLD, TTL_BUMP);
        
        env.storage().persistent().set(&DataKey::TransCount, &0u64);
        env.storage().persistent().extend_ttl(&DataKey::TransCount, TTL_THRESHOLD, TTL_BUMP);
        
        env.storage().persistent().set(&DataKey::SignerCount, &signers.len());
        env.storage().persistent().extend_ttl(&DataKey::SignerCount, TTL_THRESHOLD, TTL_BUMP);
        
        for signer in signers.iter() {
            let key = DataKey::Signer(signer);
            env.storage().persistent().set(&key, &true);
            env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
        }
    }
    
    pub fn propose_transaction(
        env: Env,
        proposer: Address,
        to: Address,
        token: Address,
        amount: i128,
        execute_after: u64,
    ) -> u64 {
        proposer.require_auth();
        if amount <= 0 {
            panic!("Amount must be greater than zero");
        }
        
        let mut count: u64 = env.storage().persistent().get(&DataKey::TransCount).unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&DataKey::TransCount, &count);
        env.storage().persistent().extend_ttl(&DataKey::TransCount, TTL_THRESHOLD, TTL_BUMP);
        
        let tx = Transaction {
            id: count,
            proposer: proposer.clone(),
            to: to.clone(),
            token: token.clone(),
            amount,
            execute_after,
            status: TransactionStatus::Pending,
            approvals: 0,
        };
        
        let key = DataKey::Transaction(count);
        env.storage().persistent().set(&key, &tx);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
        
        env.events().publish((symbol_short!("proposed"), count), (proposer, to, amount));
        
        count
    }
    
    pub fn approve_transaction(env: Env, signer: Address, trans_id: u64) {
        signer.require_auth();
        
        let is_signer = env.storage().persistent().get(&DataKey::Signer(signer.clone())).unwrap_or(false);
        if !is_signer {
            panic!("Not a signer");
        }
        
        let key = DataKey::Transaction(trans_id);
        let mut tx: Transaction = env.storage().persistent().get(&key).expect("Transaction not found");
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
        
        if tx.status != TransactionStatus::Pending {
            panic!("Transaction not pending");
        }
        
        let approval_key = DataKey::Approval(trans_id, signer.clone());
        let already_approved = env.storage().persistent().get(&approval_key).unwrap_or(false);
        if already_approved {
            panic!("Already approved");
        }
        
        env.storage().persistent().set(&approval_key, &true);
        env.storage().persistent().extend_ttl(&approval_key, TTL_THRESHOLD, TTL_BUMP);
        
        tx.approvals += 1;
        env.storage().persistent().set(&key, &tx);
        
        env.events().publish((symbol_short!("approved"), trans_id), signer);
    }
    
    pub fn execute_transaction(env: Env, executor: Address, trans_id: u64) {
        let key = DataKey::Transaction(trans_id);
        let mut tx: Transaction = env.storage().persistent().get(&key).expect("Transaction not found");
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
        
        if tx.status != TransactionStatus::Pending {
            panic!("Transaction not pending");
        }
        
        let threshold: u32 = env.storage().persistent().get(&DataKey::Threshold).expect("Threshold not found");
        env.storage().persistent().extend_ttl(&DataKey::Threshold, TTL_THRESHOLD, TTL_BUMP);
        
        if tx.approvals < threshold {
            panic!("Not enough approvals");
        }
        
        if env.ledger().timestamp() < tx.execute_after {
            panic!("Time-lock active");
        }
        
        let token_client = token::Client::new(&env, &tx.token);
        token_client.transfer(&env.current_contract_address(), &tx.to, &tx.amount);
        
        tx.status = TransactionStatus::Executed;
        env.storage().persistent().set(&key, &tx);
        
        env.events().publish((symbol_short!("executed"), trans_id), executor);
    }
    
    pub fn cancel_transaction(env: Env, admin_or_proposer: Address, trans_id: u64) {
        admin_or_proposer.require_auth();
        
        let key = DataKey::Transaction(trans_id);
        let mut tx: Transaction = env.storage().persistent().get(&key).expect("Transaction not found");
        
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("Admin not found");
        
        if admin_or_proposer != admin && admin_or_proposer != tx.proposer {
            panic!("Unauthorized");
        }
        
        if tx.status != TransactionStatus::Pending {
            panic!("Transaction not pending");
        }
        
        tx.status = TransactionStatus::Cancelled;
        env.storage().persistent().set(&key, &tx);
        
        env.events().publish((symbol_short!("cancelled"), trans_id), admin_or_proposer);
    }
    
    pub fn add_signer(env: Env, admin: Address, signer: Address) {
        admin.require_auth();
        let stored_admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("Admin not found");
        if admin != stored_admin { panic!("Unauthorized"); }
        
        let key = DataKey::Signer(signer.clone());
        let is_signer = env.storage().persistent().get(&key).unwrap_or(false);
        if is_signer { panic!("Already a signer"); }
        
        env.storage().persistent().set(&key, &true);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
        
        let mut count: u32 = env.storage().persistent().get(&DataKey::SignerCount).unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&DataKey::SignerCount, &count);
        env.storage().persistent().extend_ttl(&DataKey::SignerCount, TTL_THRESHOLD, TTL_BUMP);
        
        env.events().publish((Symbol::new(&env, "signer_added"),), signer);
    }
    
    pub fn remove_signer(env: Env, admin: Address, signer: Address) {
        admin.require_auth();
        let stored_admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("Admin not found");
        if admin != stored_admin { panic!("Unauthorized"); }
        
        let key = DataKey::Signer(signer.clone());
        let is_signer = env.storage().persistent().get(&key).unwrap_or(false);
        if !is_signer { panic!("Not a signer"); }
        
        env.storage().persistent().set(&key, &false);
        
        let mut count: u32 = env.storage().persistent().get(&DataKey::SignerCount).unwrap_or(0);
        count -= 1;
        
        let threshold: u32 = env.storage().persistent().get(&DataKey::Threshold).unwrap_or(0);
        if count < threshold {
            panic!("Signers < threshold");
        }
        
        env.storage().persistent().set(&DataKey::SignerCount, &count);
        env.storage().persistent().extend_ttl(&DataKey::SignerCount, TTL_THRESHOLD, TTL_BUMP);
        
        env.events().publish((Symbol::new(&env, "signer_removed"),), signer);
    }
    
    pub fn update_threshold(env: Env, admin: Address, threshold: u32) {
        admin.require_auth();
        let stored_admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("Admin not found");
        if admin != stored_admin { panic!("Unauthorized"); }
        
        let count: u32 = env.storage().persistent().get(&DataKey::SignerCount).unwrap_or(0);
        if threshold == 0 || threshold > count {
            panic!("Invalid threshold");
        }
        
        env.storage().persistent().set(&DataKey::Threshold, &threshold);
        env.storage().persistent().extend_ttl(&DataKey::Threshold, TTL_THRESHOLD, TTL_BUMP);
        
        env.events().publish((Symbol::new(&env, "threshold_updated"),), threshold);
    }
    
    pub fn get_transaction(env: Env, trans_id: u64) -> Transaction {
        let key = DataKey::Transaction(trans_id);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
        env.storage().persistent().get(&key).expect("Transaction not found")
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::{testutils::{Address as _}, Address, Env, Vec};

    #[test]
    fn test_initialize_and_prevent_reinit() {
        let env = Env::default();
        let contract_id = env.register_contract(None, MultiSigContract);
        let client = MultiSigContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let s1 = Address::generate(&env);
        let s2 = Address::generate(&env);
        
        let signers = Vec::from_slice(&env, &[s1.clone(), s2.clone()]);
        client.initialize(&admin, &signers, &2);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.initialize(&admin, &signers, &2);
        }));
        assert!(result.is_err());
    }
}
