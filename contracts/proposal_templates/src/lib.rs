#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Bytes, BytesN, Env,
    Map, Symbol, TryFromVal, Val,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotAdmin = 2,
    InvalidSchemaHash = 3,
    TemplateNotFound = 4,
    NotInitialized = 5,
    ProposalNotFound = 6,
    InvalidParams = 7,
    ProposalOverflow = 8,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemplateType {
    UpdateFee,
    UpdateAutoRelease,
    AddAsset,
    UpdateAdmin,
    UpdateKycRequirement,
    UpdateVelocityLimit,
    TreasuryAllocation,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateRecord {
    pub template_type: TemplateType,
    pub param_schema_hash: BytesN<32>,
    pub registered_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalRecord {
    pub id: u32,
    pub proposer: Address,
    pub template_type: TemplateType,
    pub params: Map<Symbol, Val>,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    ProposalCount,
    Template(TemplateType),
    Proposal(u32),
    TemplateHash(Address, Symbol),
}

#[contract]
pub struct ProposalTemplatesContract;

#[contractimpl]
impl ProposalTemplatesContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::ProposalCount, &0u32);
        Ok(())
    }

    pub fn register_template(
        env: Env,
        template_type: TemplateType,
        param_schema_hash: BytesN<32>,
    ) -> Result<(), Error> {
        Self::require_admin(&env)?;

        let expected = Self::expected_schema_hash(&env, &template_type);
        if param_schema_hash != expected {
            return Err(Error::InvalidSchemaHash);
        }

        let record = TemplateRecord {
            template_type: template_type.clone(),
            param_schema_hash,
            registered_at: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Template(template_type), &record);
        Ok(())
    }

    pub fn create_from_template(
        env: Env,
        proposer: Address,
        template_type: TemplateType,
        params: Map<Symbol, Val>,
    ) -> Result<u32, Error> {
        proposer.require_auth();
        Self::require_initialized(&env)?;

        let template = Self::get_template(env.clone(), template_type.clone())?;
        if template.param_schema_hash != Self::expected_schema_hash(&env, &template_type) {
            return Err(Error::InvalidSchemaHash);
        }

        Self::validate_params(&env, &template_type, &params)?;

        let mut count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCount)
            .unwrap_or(0);
        count = count.checked_add(1).ok_or(Error::ProposalOverflow)?;

        let record = ProposalRecord {
            id: count,
            proposer: proposer.clone(),
            template_type: template_type.clone(),
            params,
            created_at: env.ledger().timestamp(),
        };

        env.storage()
            .instance()
            .set(&DataKey::ProposalCount, &count);
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(count), &record);

        env.events().publish(
            (
                symbol_short!("tmpl"),
                Symbol::new(&env, "proposal_created_from_template"),
                count,
            ),
            (proposer, template_type),
        );

        Ok(count)
    }

    pub fn add_template(
        env: Env,
        admin: Address,
        target: Address,
        function: Symbol,
        args_schema_hash: BytesN<32>,
    ) -> Result<(), Error> {
        Self::require_initialized(&env)?;
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotAdmin)?;
        if stored_admin != admin {
            return Err(Error::NotAdmin);
        }
        admin.require_auth();

        env.storage()
            .persistent()
            .set(&DataKey::TemplateHash(target, function), &args_schema_hash);
        Ok(())
    }

    pub fn get_template(env: Env, template_type: TemplateType) -> Result<TemplateRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Template(template_type))
            .ok_or(Error::TemplateNotFound)
    }

    pub fn get_template_hash(
        env: Env,
        target: Address,
        function: Symbol,
    ) -> Result<Option<BytesN<32>>, Error> {
        Ok(env
            .storage()
            .persistent()
            .get(&DataKey::TemplateHash(target, function)))
    }

    pub fn get_proposal(env: Env, proposal_id: u32) -> Result<ProposalRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(Error::ProposalNotFound)
    }

    fn require_initialized(env: &Env) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }
        Ok(())
    }

    fn require_admin(env: &Env) -> Result<Address, Error> {
        Self::require_initialized(env)?;
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotAdmin)?;
        admin.require_auth();
        Ok(admin)
    }

    fn expected_schema_hash(env: &Env, template_type: &TemplateType) -> BytesN<32> {
        let schema = Self::schema_descriptor(template_type);
        let bytes = Bytes::from_slice(env, schema.as_bytes());
        env.crypto().sha256(&bytes).into()
    }

    fn schema_descriptor(template_type: &TemplateType) -> &'static str {
        match template_type {
            TemplateType::UpdateFee => "UpdateFee:fee_bps:u32",
            TemplateType::UpdateAutoRelease => "UpdateAutoRelease:auto_release_secs:u64",
            TemplateType::AddAsset => "AddAsset:asset:Address",
            TemplateType::UpdateAdmin => "UpdateAdmin:new_admin:Address",
            TemplateType::UpdateKycRequirement => "UpdateKycRequirement:kyc_required:bool",
            TemplateType::UpdateVelocityLimit => "UpdateVelocityLimit:velocity_limit:i128",
            TemplateType::TreasuryAllocation => "TreasuryAllocation:recipient:Address,amount:i128",
        }
    }

    fn validate_params(
        env: &Env,
        template_type: &TemplateType,
        params: &Map<Symbol, Val>,
    ) -> Result<(), Error> {
        match template_type {
            TemplateType::UpdateFee => {
                Self::require_exact_len(params, 1)?;
                let fee_bps: u32 = Self::required_param(env, params, "fee_bps")?;
                if fee_bps > 10_000 {
                    return Err(Error::InvalidParams);
                }
            }
            TemplateType::UpdateAutoRelease => {
                Self::require_exact_len(params, 1)?;
                let delay: u64 = Self::required_param(env, params, "auto_release_secs")?;
                if delay == 0 {
                    return Err(Error::InvalidParams);
                }
            }
            TemplateType::AddAsset => {
                Self::require_exact_len(params, 1)?;
                let _: Address = Self::required_param(env, params, "asset")?;
            }
            TemplateType::UpdateAdmin => {
                Self::require_exact_len(params, 1)?;
                let _: Address = Self::required_param(env, params, "new_admin")?;
            }
            TemplateType::UpdateKycRequirement => {
                Self::require_exact_len(params, 1)?;
                let _: bool = Self::required_param(env, params, "kyc_required")?;
            }
            TemplateType::UpdateVelocityLimit => {
                Self::require_exact_len(params, 1)?;
                let limit: i128 = Self::required_param(env, params, "velocity_limit")?;
                if limit <= 0 {
                    return Err(Error::InvalidParams);
                }
            }
            TemplateType::TreasuryAllocation => {
                Self::require_exact_len(params, 2)?;
                let _: Address = Self::required_param(env, params, "recipient")?;
                let amount: i128 = Self::required_param(env, params, "amount")?;
                if amount <= 0 {
                    return Err(Error::InvalidParams);
                }
            }
        }
        Ok(())
    }

    fn require_exact_len(params: &Map<Symbol, Val>, expected: u32) -> Result<(), Error> {
        if params.len() != expected {
            return Err(Error::InvalidParams);
        }
        Ok(())
    }

    fn required_param<T>(env: &Env, params: &Map<Symbol, Val>, key: &str) -> Result<T, Error>
    where
        T: TryFromVal<Env, Val>,
    {
        let sym = Symbol::new(env, key);
        let raw: Val = params.get(sym).ok_or(Error::InvalidParams)?;
        T::try_from_val(env, &raw).map_err(|_| Error::InvalidParams)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::IntoVal;

    fn setup() -> (Env, Address, ProposalTemplatesContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, ProposalTemplatesContract);
        let client = ProposalTemplatesContractClient::new(&env, &contract_id);
        client.initialize(&admin);

        (env, admin, client)
    }

    fn schema_hash_for(env: &Env, template_type: &TemplateType) -> BytesN<32> {
        let descriptor = match template_type {
            TemplateType::UpdateFee => "UpdateFee:fee_bps:u32",
            TemplateType::UpdateAutoRelease => "UpdateAutoRelease:auto_release_secs:u64",
            TemplateType::AddAsset => "AddAsset:asset:Address",
            TemplateType::UpdateAdmin => "UpdateAdmin:new_admin:Address",
            TemplateType::UpdateKycRequirement => "UpdateKycRequirement:kyc_required:bool",
            TemplateType::UpdateVelocityLimit => "UpdateVelocityLimit:velocity_limit:i128",
            TemplateType::TreasuryAllocation => "TreasuryAllocation:recipient:Address,amount:i128",
        };

        let bytes = Bytes::from_slice(env, descriptor.as_bytes());
        env.crypto().sha256(&bytes).into()
    }

    #[test]
    fn test_error_already_initialized() {
        let (_env, admin, client) = setup();
        let res = client.try_initialize(&admin);
        assert_eq!(res, Err(Ok(Error::AlreadyInitialized)));
    }

    #[test]
    fn test_error_not_initialized() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, ProposalTemplatesContract);
        let client = ProposalTemplatesContractClient::new(&env, &contract_id);

        let template = TemplateType::UpdateFee;
        let schema_hash = schema_hash_for(&env, &template);
        let res = client.try_register_template(&template, &schema_hash);
        assert_eq!(res, Err(Ok(Error::NotInitialized)));
    }

    #[test]
    fn test_error_not_admin() {
        let (env, _admin, client) = setup();
        let stranger = Address::generate(&env);
        let target = Address::generate(&env);
        let func = Symbol::new(&env, "test");
        let hash = BytesN::<32>::from_array(&env, &[0u8; 32]);

        let res = client.try_add_template(&stranger, &target, &func, &hash);
        assert_eq!(res, Err(Ok(Error::NotAdmin)));
    }

    #[test]
    fn test_error_invalid_schema_hash() {
        let (env, _admin, client) = setup();
        let template = TemplateType::UpdateFee;
        let bad_hash = BytesN::<32>::from_array(&env, &[99u8; 32]);

        let res = client.try_register_template(&template, &bad_hash);
        assert_eq!(res, Err(Ok(Error::InvalidSchemaHash)));
    }

    #[test]
    fn test_error_template_not_found() {
        let (_env, _admin, client) = setup();
        let res = client.try_get_template(&TemplateType::AddAsset);
        assert_eq!(res, Err(Ok(Error::TemplateNotFound)));
    }

    #[test]
    fn test_error_proposal_not_found() {
        let (_env, _admin, client) = setup();
        let res = client.try_get_proposal(&999);
        assert_eq!(res, Err(Ok(Error::ProposalNotFound)));
    }

    #[test]
    fn test_error_invalid_params() {
        let (env, _admin, client) = setup();
        let template = TemplateType::UpdateFee;
        let schema_hash = schema_hash_for(&env, &template);
        client.register_template(&template, &schema_hash);

        let proposer = Address::generate(&env);

        // Missing required param / wrong key
        let mut bad_params = Map::<Symbol, Val>::new(&env);
        bad_params.set(Symbol::new(&env, "wrong_key"), 250u32.into_val(&env));
        let res = client.try_create_from_template(&proposer, &template, &bad_params);
        assert_eq!(res, Err(Ok(Error::InvalidParams)));

        // Invalid fee bps (> 10000)
        let mut excessive_fee = Map::<Symbol, Val>::new(&env);
        excessive_fee.set(Symbol::new(&env, "fee_bps"), 15_000u32.into_val(&env));
        let res2 = client.try_create_from_template(&proposer, &template, &excessive_fee);
        assert_eq!(res2, Err(Ok(Error::InvalidParams)));
    }

    #[test]
    fn test_successful_flow() {
        let (env, admin, client) = setup();

        let template = TemplateType::UpdateFee;
        let schema_hash = schema_hash_for(&env, &template);
        assert!(client.try_register_template(&template, &schema_hash).is_ok());

        let stored = client.get_template(&template);
        assert_eq!(stored.template_type, template);
        assert_eq!(stored.param_schema_hash, schema_hash);

        let proposer = Address::generate(&env);
        let mut params = Map::<Symbol, Val>::new(&env);
        params.set(Symbol::new(&env, "fee_bps"), 250u32.into_val(&env));

        let proposal_id = client.create_from_template(&proposer, &template, &params);
        assert_eq!(proposal_id, 1);

        let proposal = client.get_proposal(&proposal_id);
        assert_eq!(proposal.id, 1);
        assert_eq!(proposal.proposer, proposer);

        let target = Address::generate(&env);
        let func = Symbol::new(&env, "update");
        assert!(client.try_add_template(&admin, &target, &func, &schema_hash).is_ok());
        assert_eq!(client.get_template_hash(&target, &func), Some(schema_hash));
    }
}
