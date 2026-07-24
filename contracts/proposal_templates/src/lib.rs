#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Bytes, BytesN, Env, Map, Symbol,
    TryFromVal, Val,
};

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
    Admin,
    ProposalCount,
    Template(TemplateType),
    Proposal(u32),
}

#[contract]
pub struct ProposalTemplatesContract;

#[contractimpl]
impl ProposalTemplatesContract {
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::ProposalCount, &0u32);
    }

    pub fn register_template(env: Env, template_type: TemplateType, param_schema_hash: BytesN<32>) {
        Self::require_admin(&env);

        let expected = Self::expected_schema_hash(&env, &template_type);
        if param_schema_hash != expected {
            panic!("invalid schema hash");
        }

        let record = TemplateRecord {
            template_type: template_type.clone(),
            param_schema_hash,
            registered_at: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Template(template_type), &record);
    }

    pub fn create_from_template(
        env: Env,
        proposer: Address,
        template_type: TemplateType,
        params: Map<Symbol, Val>,
    ) -> u32 {
        proposer.require_auth();
        Self::require_initialized(&env);

        let template = Self::get_template(env.clone(), template_type.clone());
        if template.param_schema_hash != Self::expected_schema_hash(&env, &template_type) {
            panic!("template schema mismatch");
        }

        Self::validate_params(&env, &template_type, &params);

        let mut count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCount)
            .unwrap_or(0);
        count = count.checked_add(1).expect("proposal overflow");

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

        count
    }

    pub fn get_template(env: Env, template_type: TemplateType) -> TemplateRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Template(template_type))
            .expect("template not found")
    }

    pub fn get_proposal(env: Env, proposal_id: u32) -> ProposalRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .expect("proposal not found")
    }

    fn require_initialized(env: &Env) {
        if !env.storage().instance().has(&DataKey::Admin) {
            panic!("not initialized");
        }
    }

    fn require_admin(env: &Env) {
        Self::require_initialized(env);
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("admin missing");
        admin.require_auth();
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

    fn validate_params(env: &Env, template_type: &TemplateType, params: &Map<Symbol, Val>) {
        match template_type {
            TemplateType::UpdateFee => {
                Self::require_exact_len(params, 1);
                let fee_bps: u32 = Self::required_param(env, params, "fee_bps");
                if fee_bps > 10_000 {
                    panic!("invalid fee bps");
                }
            }
            TemplateType::UpdateAutoRelease => {
                Self::require_exact_len(params, 1);
                let delay: u64 = Self::required_param(env, params, "auto_release_secs");
                if delay == 0 {
                    panic!("invalid auto release delay");
                }
            }
            TemplateType::AddAsset => {
                Self::require_exact_len(params, 1);
                let _: Address = Self::required_param(env, params, "asset");
            }
            TemplateType::UpdateAdmin => {
                Self::require_exact_len(params, 1);
                let _: Address = Self::required_param(env, params, "new_admin");
            }
            TemplateType::UpdateKycRequirement => {
                Self::require_exact_len(params, 1);
                let _: bool = Self::required_param(env, params, "kyc_required");
            }
            TemplateType::UpdateVelocityLimit => {
                Self::require_exact_len(params, 1);
                let limit: i128 = Self::required_param(env, params, "velocity_limit");
                if limit <= 0 {
                    panic!("invalid velocity limit");
                }
            }
            TemplateType::TreasuryAllocation => {
                Self::require_exact_len(params, 2);
                let _: Address = Self::required_param(env, params, "recipient");
                let amount: i128 = Self::required_param(env, params, "amount");
                if amount <= 0 {
                    panic!("invalid treasury allocation");
                }
            }
        }
    }

    fn require_exact_len(params: &Map<Symbol, Val>, expected: u32) {
        if params.len() != expected {
            panic!("invalid param set");
        }
    }

    fn required_param<T>(env: &Env, params: &Map<Symbol, Val>, key: &str) -> T
    where
        T: TryFromVal<Env, Val>,
    {
        let sym = Symbol::new(env, key);
        let raw: Val = params.get(sym).expect("missing required param");
        T::try_from_val(env, &raw).expect("invalid param type")
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
        env.crypto().sha256(&bytes)
    }

    #[test]
    fn test_register_template() {
        let (env, _admin, client) = setup();

        let template = TemplateType::UpdateFee;
        let schema_hash = schema_hash_for(&env, &template);
        client.register_template(&template, &schema_hash);

        let stored = client.get_template(&template);
        assert_eq!(stored.template_type, template);
        assert_eq!(stored.param_schema_hash, schema_hash);
    }

    #[test]
    fn test_create_valid_proposal() {
        let (env, _admin, client) = setup();

        let template = TemplateType::UpdateFee;
        let schema_hash = schema_hash_for(&env, &template);
        client.register_template(&template, &schema_hash);

        let proposer = Address::generate(&env);
        let mut params = Map::<Symbol, Val>::new(&env);
        params.set(Symbol::new(&env, "fee_bps"), 250u32.into_val(&env));

        let proposal_id = client.create_from_template(&proposer, &template, &params);
        assert_eq!(proposal_id, 1);

        let proposal = client.get_proposal(&proposal_id);
        assert_eq!(proposal.id, proposal_id);
        assert_eq!(proposal.proposer, proposer);
        assert_eq!(proposal.template_type, template);
    }

    #[test]
    #[should_panic(expected = "missing required param")]
    fn test_invalid_params_rejection() {
        let (env, _admin, client) = setup();

        let template = TemplateType::UpdateFee;
        let schema_hash = schema_hash_for(&env, &template);
        client.register_template(&template, &schema_hash);

        let proposer = Address::generate(&env);
        let mut params = Map::<Symbol, Val>::new(&env);
        params.set(Symbol::new(&env, "wrong_key"), 250u32.into_val(&env));

        client.create_from_template(&proposer, &template, &params);
    }
}
