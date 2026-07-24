#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, BytesN, Env,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    MarketNotFound = 3,
    MarketAlreadyResolved = 4,
    MarketNotResolved = 5,
    InvalidAmount = 6,
    NotAdmin = 7,
    ResolutionNotReady = 8,
    NoWinnings = 9,
}

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketRecord {
    pub id: u32,
    pub creator: Address,
    pub learner: Address,
    pub goal_description_hash: BytesN<32>,
    pub resolution_date: u64,
    pub token: Address,
    pub yes_pool: i128,
    pub no_pool: i128,
    pub resolved: bool,
    pub outcome: Option<bool>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BetRecord {
    pub bettor: Address,
    pub market_id: u32,
    pub outcome: bool,
    pub amount: i128,
    pub claimed: bool,
}

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    MarketCount,
    Market(u32),
    Bet(Address, u32),
    BettorMarkets(Address),
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PLATFORM_FEE_BPS: i128 = 200; // 2%

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct PredictionMarket;

#[contractimpl]
impl PredictionMarket {
    /// Initialize the prediction market contract
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MarketCount, &0u32);
    }

    /// Create a new prediction market
    pub fn create_market(
        env: Env,
        creator: Address,
        learner: Address,
        goal_description_hash: BytesN<32>,
        resolution_date: u64,
        token: Address,
    ) -> u32 {
        creator.require_auth();

        let now = env.ledger().timestamp();
        if resolution_date <= now {
            panic!("resolution date must be in future");
        }

        let market_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MarketCount)
            .unwrap_or(0);
        let market_id = market_count + 1;

        let market = MarketRecord {
            id: market_id,
            creator: creator.clone(),
            learner: learner.clone(),
            goal_description_hash,
            resolution_date,
            token,
            yes_pool: 0,
            no_pool: 0,
            resolved: false,
            outcome: None,
        };

        env.storage()
            .instance()
            .set(&DataKey::Market(market_id), &market);
        env.storage()
            .instance()
            .set(&DataKey::MarketCount, &market_id);

        env.events()
            .publish((symbol_short!("mkt_crt"),), (market_id, creator, learner));

        market_id
    }

    /// Place a bet on market outcome
    pub fn place_bet(env: Env, bettor: Address, market_id: u32, outcome: bool, amount: i128) {
        if amount <= 0 {
            panic!("invalid amount");
        }

        bettor.require_auth();

        let mut market: MarketRecord = env
            .storage()
            .instance()
            .get(&DataKey::Market(market_id))
            .expect("market not found");

        if market.resolved {
            panic!("market already resolved");
        }

        let now = env.ledger().timestamp();
        if now >= market.resolution_date {
            panic!("market resolution date passed");
        }

        // Transfer tokens from bettor to contract
        let token_client = token::Client::new(&env, &market.token);
        token_client.transfer(&bettor, &env.current_contract_address(), &amount);

        // Update pools
        if outcome {
            market.yes_pool += amount;
        } else {
            market.no_pool += amount;
        }

        env.storage()
            .instance()
            .set(&DataKey::Market(market_id), &market);

        // Record bet
        let bet = BetRecord {
            bettor: bettor.clone(),
            market_id,
            outcome,
            amount,
            claimed: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Bet(bettor.clone(), market_id), &bet);

        env.events().publish(
            (symbol_short!("bet_pl"),),
            (bettor, market_id, outcome, amount),
        );
    }

    /// Resolve market with outcome (admin/oracle only)
    pub fn resolve_market(env: Env, market_id: u32, outcome: bool) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        let mut market: MarketRecord = env
            .storage()
            .instance()
            .get(&DataKey::Market(market_id))
            .expect("market not found");

        if market.resolved {
            panic!("market already resolved");
        }

        let now = env.ledger().timestamp();
        if now < market.resolution_date {
            panic!("resolution date not reached");
        }

        market.resolved = true;
        market.outcome = Some(outcome);

        env.storage()
            .instance()
            .set(&DataKey::Market(market_id), &market);

        env.events()
            .publish((symbol_short!("mkt_res"),), (market_id, outcome));
    }

    /// Claim winnings from resolved market
    pub fn claim_winnings(env: Env, bettor: Address, market_id: u32) {
        bettor.require_auth();

        let market: MarketRecord = env
            .storage()
            .instance()
            .get(&DataKey::Market(market_id))
            .expect("market not found");

        if !market.resolved {
            panic!("market not resolved");
        }

        let mut bet: BetRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bet(bettor.clone(), market_id))
            .expect("bet not found");

        if bet.claimed {
            panic!("winnings already claimed");
        }

        let winning_outcome = market.outcome.expect("outcome not set");

        if bet.outcome != winning_outcome {
            panic!("bet did not win");
        }

        // Calculate winnings
        let losing_pool = if winning_outcome {
            market.no_pool
        } else {
            market.yes_pool
        };

        let winning_pool = if winning_outcome {
            market.yes_pool
        } else {
            market.no_pool
        };

        let platform_fee = (losing_pool * PLATFORM_FEE_BPS) / 10_000;
        let net_winnings = losing_pool - platform_fee;
        let share = (net_winnings * bet.amount) / winning_pool;
        let total_payout = bet.amount + share;

        // Transfer winnings to bettor
        let token_client = token::Client::new(&env, &market.token);
        token_client.transfer(&env.current_contract_address(), &bettor, &total_payout);

        // Mark bet as claimed
        bet.claimed = true;
        env.storage()
            .persistent()
            .set(&DataKey::Bet(bettor.clone(), market_id), &bet);

        env.events().publish(
            (symbol_short!("win_clm"),),
            (bettor, market_id, total_payout),
        );
    }

    /// Cancel market and refund all bets (admin only)
    pub fn cancel_market(env: Env, market_id: u32) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        let market: MarketRecord = env
            .storage()
            .instance()
            .get(&DataKey::Market(market_id))
            .expect("market not found");

        if market.resolved {
            panic!("cannot cancel resolved market");
        }

        // In production, would iterate through all bets and refund
        // For now, just mark as resolved with no outcome
        let mut updated_market = market.clone();
        updated_market.resolved = true;

        env.storage()
            .instance()
            .set(&DataKey::Market(market_id), &updated_market);

        env.events().publish((symbol_short!("mkt_can"),), market_id);
    }

    /// Get market record
    pub fn get_market(env: Env, id: u32) -> MarketRecord {
        env.storage()
            .instance()
            .get(&DataKey::Market(id))
            .expect("market not found")
    }

    /// Get market odds
    pub fn get_odds(env: Env, market_id: u32) -> (i128, i128) {
        let market: MarketRecord = env
            .storage()
            .instance()
            .get(&DataKey::Market(market_id))
            .expect("market not found");

        (market.yes_pool, market.no_pool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_market() {
        let env = Env::default();
        let contract_id = env.register_contract(None, PredictionMarket);
        let client = PredictionMarketClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let creator = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = Address::generate(&env);
        let hash = BytesN::<32>::from_array(&env, &[0u8; 32]);

        env.mock_all_auths();
        client.initialize(&admin);

        let market_id = client.create_market(&creator, &learner, &hash, &1000, &token);
        assert_eq!(market_id, 1);
    }

    #[test]
    fn test_place_bet() {
        let env = Env::default();
        let contract_id = env.register_contract(None, PredictionMarket);
        let client = PredictionMarketClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let creator = Address::generate(&env);
        let learner = Address::generate(&env);
        let bettor = Address::generate(&env);
        let token = Address::generate(&env);
        let hash = BytesN::<32>::from_array(&env, &[0u8; 32]);

        env.mock_all_auths();
        client.initialize(&admin);

        let market_id = client.create_market(&creator, &learner, &hash, &1000, &token);
        client.place_bet(&bettor, &market_id, &true, &100);

        let (yes_pool, no_pool) = client.get_odds(&market_id);
        assert_eq!(yes_pool, 100);
        assert_eq!(no_pool, 0);
    }

    #[test]
    fn test_resolve_market() {
        let env = Env::default();
        let contract_id = env.register_contract(None, PredictionMarket);
        let client = PredictionMarketClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let creator = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = Address::generate(&env);
        let hash = BytesN::<32>::from_array(&env, &[0u8; 32]);

        env.mock_all_auths();
        client.initialize(&admin);

        let market_id = client.create_market(&creator, &learner, &hash, &100, &token);

        // Advance ledger past resolution date
        env.ledger().set_timestamp(101);

        client.resolve_market(&market_id, &true);
        let market = client.get_market(&market_id);
        assert!(market.resolved);
        assert_eq!(market.outcome, Some(true));
    }

    #[test]
    #[should_panic(expected = "resolution date must be in future")]
    fn test_invalid_resolution_date() {
        let env = Env::default();
        let contract_id = env.register_contract(None, PredictionMarket);
        let client = PredictionMarketClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let creator = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = Address::generate(&env);
        let hash = BytesN::<32>::from_array(&env, &[0u8; 32]);

        env.mock_all_auths();
        client.initialize(&admin);

        client.create_market(&creator, &learner, &hash, &0, &token);
    }
}
