#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short,
    token, Address, Env, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized      = 1,
    NotInitialized          = 2,
    Unauthorized            = 3,
    InvalidRange            = 4,
    NoData                  = 5,
    MismatchDetected        = 6,
    InvalidAmount           = 7,
    DuplicateRecord         = 8,
    Overflow                = 9,
    Underflow               = 10,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const BPS_DENOM: i128 = 10_000;
const REFERRAL_REWARD_MENTOR: i128 = 50 * 10_000_000;
const REFERRAL_REWARD_LEARNER: i128 = 20 * 10_000_000;
const INSURANCE_YIELD_BPS: i128 = 10;

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

/// Origin of a fee distribution record used for reconciliation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq, Copy)]
#[repr(u32)]
pub enum LedgerSource {
    EscrowFee       = 1,
    ReferralReward  = 2,
    ReferralFeeDist = 3,
    InsuranceYield  = 4,
    InsuranceClaim  = 5,
    TreasuryDeposit = 6,
    TreasuryAlloc   = 7,
    TreasuryDistrib = 8,
    BuybackBurn     = 9,
}

/// A single bookkeeping "ledger line" used to reconstruct historical state.
///
/// Each record represents a single atomic accounting movement: a fee
/// collected, a referral reward minted, an insurance claim paid, etc.
/// Records are append-only and immutable once written, forming the
/// authoritative "input tape" that the reconciliation engine walks.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerRecord {
    pub id:          u64,
    pub source:      LedgerSource,
    pub token:       Address,
    /// +ve = funds flowing *into* treasury / pool / referral pot,
    /// -ve = funds flowing *out* (payment, claim, allocation, burn, ...).
    pub amount:      i128,
    /// Secondary amount: bps used for fee calc, multiplier for referrals,
    /// insurance yield basis points, etc. — used for re-verification.
    pub aux_bps:     u32,
    /// Gross amount the fee/reward was calculated from (for recomputation).
    pub gross_base:  i128,
    /// Associated account: mentor, referrer, insurance provider, etc.
    pub account:     Address,
    /// Arbitrary reference key (escrow id, proposal id, op id, ...) as Symbol.
    pub reference:   Symbol,
    pub timestamp:   u64,
}

/// Output of `validate_fee_distribution` — one line per validated escrow
/// fee release with recomputed vs recorded values.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeValidationEntry {
    pub reference:    Symbol,
    pub gross_amount: i128,
    pub fee_bps:      u32,
    pub expected_fee: i128,
    pub recorded_fee: i128,
    pub expected_net: i128,
    pub recorded_net: i128,
    pub fee_matches:  bool,
    pub net_matches:  bool,
    pub sum_matches:  bool,
}

/// Output of `validate_referral_rewards`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralValidationEntry {
    pub referrer:      Address,
    pub referee_type:  u32, // 0 = mentor, 1 = learner, 2 = fee-based
    pub base_reward:   i128,
    pub multiplier_bps: u32,
    pub expected:      i128,
    pub recorded:      i128,
    pub matches:       bool,
    pub within_lifetime_cap: bool,
    pub within_global_cap:  bool,
}

/// One row of the insurance cross-check: expected vs recorded pool delta.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsuranceValidationEntry {
    pub provider:           Address,
    pub expected_shares:    i128,
    pub recorded_shares:    i128,
    pub expected_pool_delta: i128,
    pub recorded_pool_delta: i128,
    pub shares_match:       bool,
    pub pool_match:         bool,
}

/// Outcome of a balance-consistency probe across contracts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsistencyProbe {
    pub contract:     Address,
    pub token:        Address,
    pub actual_balance: i128,
    pub computed_balance: i128,
    pub difference:   i128,
    pub consistent:   bool,
}

/// A single accounting mismatch surfaced by `detect_mismatches`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MismatchRecord {
    pub record_id:   u64,
    pub source:      LedgerSource,
    pub field:       Symbol,
    pub expected:    i128,
    pub recorded:    i128,
    pub difference:  i128,
    pub severity:    u32, // 1=warn, 2=critical, 3=fatal
    pub description: Symbol,
}

/// The final reconciliation report returned by `generate_report`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationReport {
    pub report_id:                u64,
    pub generated_at:             u64,
    pub range_start:              u64,
    pub range_end:                u64,
    pub records_processed:        u64,
    pub total_inflows:            i128,
    pub total_outflows:           i128,
    pub net_flow:                 i128,
    pub fee_validations:          u32,
    pub fee_failures:             u32,
    pub referral_validations:     u32,
    pub referral_failures:        u32,
    pub insurance_validations:    u32,
    pub insurance_failures:       u32,
    pub consistency_probes:       u32,
    pub probes_failed:            u32,
    pub mismatches_found:         u32,
    pub critical_mismatches:      u32,
    pub treasury_balance_start:   i128,
    pub treasury_balance_end:     i128,
    pub treasury_computed_delta:  i128,
    pub reconciled:               bool,
    pub summary_tag:              Symbol, // "CLEAN" | "WARN" | "FAIL"
}

/// Historical transaction: single operation (deposit, release, claim, …)
/// registered with `register_historical` to form the on-chain audit trail.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalTx {
    pub tx_id:      u64,
    pub op:         Symbol,
    pub amount:     i128,
    pub token:      Address,
    pub sender:     Address,
    pub recipient:  Address,
    pub reference:  Symbol,
    pub timestamp:  u64,
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
    EscrowContract,
    TreasuryContract,
    ReferralContract,
    InsuranceContract,
    RecordCount,
    Record(u64),
    MismatchCount,
    Mismatch(u64),
    ReportCount,
    Report(u64),
    HistoricalCount,
    Historical(u64),
    /// Running totals (for O(1) delta checks):
    /// category → (total_inflows, total_outflows)
    RunningTotals(LedgerSource),
    /// Account-level aggregates: Address → (pending_claimed, total_credited)
    ReferrerAgg(Address),
    /// Provider-level share aggregates
    InsuranceAgg(Address),
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct ReconciliationEngine;

#[contractimpl]
impl ReconciliationEngine {
    /// -------------------------------------------------------------------
    /// Initialization
    /// -------------------------------------------------------------------

    pub fn initialize(
        env:           Env,
        admin:         Address,
        escrow:        Address,
        treasury:      Address,
        referral:      Address,
        insurance:     Address,
    ) -> Result<(), Error> {
        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().persistent().set(&DataKey::Admin,            &admin);
        env.storage().persistent().set(&DataKey::EscrowContract,   &escrow);
        env.storage().persistent().set(&DataKey::TreasuryContract, &treasury);
        env.storage().persistent().set(&DataKey::ReferralContract, &referral);
        env.storage().persistent().set(&DataKey::InsuranceContract,&insurance);
        env.storage().persistent().set(&DataKey::RecordCount,      &0u64);
        env.storage().persistent().set(&DataKey::MismatchCount,    &0u64);
        env.storage().persistent().set(&DataKey::ReportCount,      &0u64);
        env.storage().persistent().set(&DataKey::HistoricalCount,  &0u64);
        Ok(())
    }

    fn _assert_admin(env: &Env) -> Result<Address, Error> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        Ok(admin)
    }

    /// -------------------------------------------------------------------
    /// Ledger ingestion (append-only)
    /// -------------------------------------------------------------------

    /// Append a single `LedgerRecord` to the reconciliation tape.
    ///
    /// Returns the new record id.  Admin only; admin is expected to
    /// mirror events from the other contracts into this log.
    pub fn append_record(
        env:        Env,
        source:     LedgerSource,
        token:      Address,
        amount:     i128,
        aux_bps:    u32,
        gross_base: i128,
        account:    Address,
        reference:  Symbol,
    ) -> Result<u64, Error> {
        Self::_assert_admin(&env)?;
        let count: u64 = env.storage().persistent().get(&DataKey::RecordCount).unwrap_or(0);
        let id = count.checked_add(1).ok_or(Error::Overflow)?;
        let ts = env.ledger().timestamp();
        let rec = LedgerRecord {
            id, source, token: token.clone(), amount, aux_bps, gross_base,
            account: account.clone(), reference, timestamp: ts,
        };
        env.storage().persistent().set(&DataKey::Record(id), &rec);
        env.storage().persistent().set(&DataKey::RecordCount, &id);

        // Running totals
        let key = DataKey::RunningTotals(source);
        let (mut ins, mut outs): (i128, i128) = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or((0, 0));
        if amount >= 0 {
            ins = ins.checked_add(amount).ok_or(Error::Overflow)?;
        } else {
            outs = outs.checked_sub(amount).ok_or(Error::Overflow)?; // -(-x) = +x
        }
        env.storage().persistent().set(&key, &(ins, outs));

        // Source-specific accumulators
        match source {
            LedgerSource::ReferralReward | LedgerSource::ReferralFeeDist => {
                let k = DataKey::ReferrerAgg(account.clone());
                let (pending, credited): (i128, i128) =
                    env.storage().persistent().get(&k).unwrap_or((0, 0));
                let new_credited = credited.checked_add(amount.max(0)).ok_or(Error::Overflow)?;
                env.storage().persistent().set(&k, &(pending, new_credited));
            }
            LedgerSource::InsuranceYield | LedgerSource::InsuranceClaim => {
                let k = DataKey::InsuranceAgg(account.clone());
                let shares: i128 = env.storage().persistent().get(&k).unwrap_or(0);
                let new = shares.checked_add(amount).ok_or(Error::Overflow)?;
                env.storage().persistent().set(&k, &new);
            }
            _ => {}
        }

        env.events().publish(
            (symbol_short!("recon"), symbol_short!("rec_add"), id),
            (source as u32, amount, ts),
        );
        Ok(id)
    }

    pub fn get_record(env: Env, id: u64) -> Result<LedgerRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Record(id))
            .ok_or(Error::NoData)
    }

    pub fn get_record_count(env: Env) -> u64 {
        env.storage().persistent().get(&DataKey::RecordCount).unwrap_or(0)
    }

    /// -------------------------------------------------------------------
    /// 1. Fee distribution validation
    /// -------------------------------------------------------------------

    /// Recompute each escrow fee in `records` to verify:
    ///   expected_fee = floor(gross_base * aux_bps / 10_000)
    ///   expected_net = gross_base - expected_fee
    ///   recorded_fee + recorded_net == gross_base  (no dust)
    ///
    /// Returns per-escrow validation rows and counts mismatches.
    pub fn validate_fee_distributions(
        env:    Env,
        start:  u64,
        end:    u64,
    ) -> Result<Vec<FeeValidationEntry>, Error> {
        if start > end { return Err(Error::InvalidRange); }
        let count: u64 = env.storage().persistent().get(&DataKey::RecordCount).unwrap_or(0);
        let mut out = Vec::new(&env);
        for id in start..=end.min(count) {
            if let Some(rec) = env.storage().persistent().get::<_, LedgerRecord>(&DataKey::Record(id)) {
                if !matches!(rec.source, LedgerSource::EscrowFee) { continue; }

                let expected_fee = calc_platform_fee(rec.gross_base, rec.aux_bps);
                let expected_net = rec.gross_base.checked_sub(expected_fee).unwrap_or(0);
                // recorded_fee stored as +ve in amount
                let recorded_fee = rec.amount.max(0);
                // recorded_net is stored in gross_base - fee in the record's
                // second line (next record).  For single-record validation we
                // treat gross_base - recorded_fee as the recorded net:
                let recorded_net = rec.gross_base.checked_sub(recorded_fee).unwrap_or(0);

                let fee_ok = recorded_fee == expected_fee;
                let net_ok = recorded_net == expected_net;
                let sum_ok = recorded_fee
                    .checked_add(recorded_net)
                    .map_or(false, |s| s == rec.gross_base);

                if !fee_ok || !net_ok || !sum_ok {
                    Self::_raise_mismatch(
                        &env,
                        id, rec.source,
                        Symbol::new(&env, "platform_fee"),
                        expected_fee, recorded_fee,
                        if !sum_ok { 3 } else { 2 },
                        Symbol::new(&env, "fee_math"),
                    )?;
                }

                out.push_back(FeeValidationEntry {
                    reference: rec.reference,
                    gross_amount: rec.gross_base,
                    fee_bps: rec.aux_bps,
                    expected_fee,
                    recorded_fee,
                    expected_net,
                    recorded_net,
                    fee_matches: fee_ok,
                    net_matches: net_ok,
                    sum_matches: sum_ok,
                });
            }
        }
        Ok(out)
    }

    /// -------------------------------------------------------------------
    /// 2. Referral reward calculation verification
    /// -------------------------------------------------------------------

    /// Recompute each referral payout:
    ///   type=Mentor  → base = REFERRAL_REWARD_MENTOR
    ///   type=Learner → base = REFERRAL_REWARD_LEARNER
    ///   fee-based    → base = gross_base * aux_bps / 10_000
    /// Then apply multiplier (aux_bps is clamped to max, use global config
    /// provided by caller) and verify the recorded amount.
    pub fn validate_referral_rewards(
        env:                  Env,
        start:                u64,
        end:                  u64,
        max_multiplier_bps:   u32,
        max_lifetime_per_ref: i128,
        global_cap:           i128,
    ) -> Result<Vec<ReferralValidationEntry>, Error> {
        if start > end { return Err(Error::InvalidRange); }
        let count: u64 = env.storage().persistent().get(&DataKey::RecordCount).unwrap_or(0);
        let mut out = Vec::new(&env);
        let mut global_used: i128 = 0;
        let mut per_referrer_lifetime: BTreeMap_Address_i128 = BTreeMap_Address_i128::new(&env);

        for id in start..=end.min(count) {
            if let Some(rec) = env.storage().persistent().get::<_, LedgerRecord>(&DataKey::Record(id)) {
                let (ref_type, base) = match rec.source {
                    LedgerSource::ReferralReward => {
                        // aux_bps == 0 → Learner, aux_bps >= 1 → Mentor (convention).
                        if rec.aux_bps == 0 {
                            (1u32, REFERRAL_REWARD_LEARNER)
                        } else {
                            (0u32, REFERRAL_REWARD_MENTOR)
                        }
                    }
                    LedgerSource::ReferralFeeDist => {
                        let b = calc_platform_fee(rec.gross_base, rec.aux_bps);
                        (2u32, b)
                    }
                    _ => continue,
                };

                let multiplier = rec.aux_bps.max(10_000).min(max_multiplier_bps.max(10_000));
                let expected_raw = base
                    .checked_mul(multiplier as i128)
                    .and_then(|v| v.checked_div(BPS_DENOM))
                    .unwrap_or(0);

                // Lifetime cap check (per referrer)
                let prev_lt = per_referrer_lifetime.get(&rec.account).unwrap_or(0);
                let after_lt = prev_lt.saturating_add(expected_raw);
                let under_lt = after_lt <= max_lifetime_per_ref;
                let expected = if under_lt { expected_raw } else {
                    max_lifetime_per_ref.saturating_sub(prev_lt)
                };

                // Global cap
                let after_global = global_used.saturating_add(expected);
                let under_global = after_global <= global_cap;
                let expected = if under_global { expected } else {
                    global_cap.saturating_sub(global_used)
                };
                global_used = global_used.saturating_add(expected);
                per_referrer_lifetime.insert(rec.account.clone(), after_lt);

                let recorded = rec.amount.max(0);
                let ok = recorded == expected;

                if !ok {
                    Self::_raise_mismatch(
                        &env, id, rec.source,
                        Symbol::new(&env, "referral_payout"),
                        expected, recorded,
                        2, Symbol::new(&env, "ref_math"),
                    )?;
                }

                out.push_back(ReferralValidationEntry {
                    referrer: rec.account.clone(),
                    referee_type: ref_type,
                    base_reward: base,
                    multiplier_bps: multiplier,
                    expected,
                    recorded,
                    matches: ok,
                    within_lifetime_cap: under_lt,
                    within_global_cap: under_global,
                });
            }
        }
        Ok(out)
    }

    /// -------------------------------------------------------------------
    /// 3. Treasury balance consistency (cross-contract probe)
    /// -------------------------------------------------------------------

    /// Reads the actual on-chain token balance of the treasury contract
    /// and compares it to `start_balance + Σ(inflows) - Σ(outflows)` over
    /// the ledger records in `[start, end]`.
    pub fn check_treasury_consistency(
        env:           Env,
        token:         Address,
        start_balance: i128,
        start:         u64,
        end:           u64,
    ) -> Result<ConsistencyProbe, Error> {
        if start > end { return Err(Error::InvalidRange); }
        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::TreasuryContract)
            .ok_or(Error::NotInitialized)?;
        let count: u64 = env.storage().persistent().get(&DataKey::RecordCount).unwrap_or(0);
        let mut computed: i128 = start_balance;
        for id in start..=end.min(count) {
            if let Some(rec) = env.storage().persistent().get::<_, LedgerRecord>(&DataKey::Record(id)) {
                if rec.token != token { continue; }
                let include = matches!(
                    rec.source,
                    LedgerSource::EscrowFee | LedgerSource::TreasuryDeposit
                ) && rec.amount >= 0;
                let outflow = matches!(
                    rec.source,
                    LedgerSource::TreasuryAlloc | LedgerSource::TreasuryDistrib
                        | LedgerSource::BuybackBurn
                ) && rec.amount <= 0;
                if include || outflow {
                    computed = computed.checked_add(rec.amount).unwrap_or(computed);
                }
            }
        }
        let actual = token::Client::new(&env, &token).balance(&treasury);
        let diff = actual.checked_sub(computed).unwrap_or(0);
        let consistent = diff == 0;
        if !consistent {
            // Mismatch at treasury level — critical
            Self::_raise_mismatch_raw(
                &env,
                0, LedgerSource::TreasuryDeposit,
                Symbol::new(&env, "treasury_balance"),
                computed, actual, 3,
                Symbol::new(&env, "bal_mismatch"),
            )?;
        }
        Ok(ConsistencyProbe {
            contract: treasury,
            token: token.clone(),
            actual_balance: actual,
            computed_balance: computed,
            difference: diff,
            consistent,
        })
    }

    /// Cross-check insurance: Σ(shares per provider) == recorded pool balance.
    pub fn check_insurance_pool_consistency(
        env:              Env,
        recorded_pool:    i128,
        providers:        Vec<Address>,
    ) -> Result<InsuranceValidationEntry, Error> {
        let mut shares_total: i128 = 0;
        for provider in providers.iter() {
            let s: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::InsuranceAgg(provider.clone()))
                .unwrap_or(0);
            shares_total = shares_total.checked_add(s).unwrap_or(shares_total);
        }
        let m = shares_total == recorded_pool;
        if !m {
            Self::_raise_mismatch_raw(
                &env,
                0, LedgerSource::InsuranceYield,
                Symbol::new(&env, "pool_vs_shares"),
                shares_total, recorded_pool, 2,
                Symbol::new(&env, "ins_pool_mismatch"),
            )?;
        }
        Ok(InsuranceValidationEntry {
            provider: Address::new(&env), // placeholder — aggregate
            expected_shares: shares_total,
            recorded_shares: shares_total,
            expected_pool_delta: shares_total,
            recorded_pool_delta: recorded_pool,
            shares_match: true,
            pool_match: m,
        })
    }

    /// -------------------------------------------------------------------
    /// 4. Accounting mismatch detection engine
    /// -------------------------------------------------------------------

    /// Walk all records in `[start, end]` and surface structural mismatches
    /// that are orthogonal to the per-category validators:
    ///   • negative zero amounts with invalid sign direction
    ///   • bps > 10 000 (invalid)
    ///   • zero-gross with non-zero fee (unpossible)
    ///   • duplicate (reference, source) pairs (double-counting guard)
    pub fn detect_mismatches(
        env:   Env,
        start: u64,
        end:   u64,
    ) -> Result<Vec<MismatchRecord>, Error> {
        if start > end { return Err(Error::InvalidRange); }
        let count: u64 = env.storage().persistent().get(&DataKey::RecordCount).unwrap_or(0);
        let mut seen: Vec<(Symbol, u32)> = Vec::new(&env);
        let mut out: Vec<MismatchRecord> = Vec::new(&env);

        for id in start..=end.min(count) {
            if let Some(rec) = env.storage().persistent().get::<_, LedgerRecord>(&DataKey::Record(id)) {
                // BPS cap
                if rec.aux_bps > 10_000 {
                    let m = MismatchRecord {
                        record_id: id, source: rec.source,
                        field: Symbol::new(&env, "aux_bps"),
                        expected: 10_000, recorded: rec.aux_bps as i128,
                        difference: rec.aux_bps as i128 - 10_000,
                        severity: 2,
                        description: Symbol::new(&env, "bps_over_cap"),
                    };
                    out.push_back(m.clone());
                    Self::_push_mismatch(&env, m)?;
                }
                // zero gross with non-zero fee
                if rec.gross_base == 0 && rec.amount != 0 {
                    let m = MismatchRecord {
                        record_id: id, source: rec.source,
                        field: Symbol::new(&env, "gross_base"),
                        expected: 0, recorded: rec.amount,
                        difference: rec.amount, severity: 3,
                        description: Symbol::new(&env, "zero_gross_fee"),
                    };
                    out.push_back(m.clone());
                    Self::_push_mismatch(&env, m)?;
                }
                // sign sanity: EscrowFee/TreasuryDeposit → +ve; Alloc/Claim → -ve
                let pos_expected = matches!(
                    rec.source,
                    LedgerSource::EscrowFee | LedgerSource::ReferralReward
                        | LedgerSource::ReferralFeeDist | LedgerSource::InsuranceYield
                        | LedgerSource::TreasuryDeposit
                );
                let neg_expected = matches!(
                    rec.source,
                    LedgerSource::InsuranceClaim | LedgerSource::TreasuryAlloc
                        | LedgerSource::TreasuryDistrib | LedgerSource::BuybackBurn
                );
                if pos_expected && rec.amount < 0 {
                    let m = MismatchRecord {
                        record_id: id, source: rec.source,
                        field: Symbol::new(&env, "sign"),
                        expected: 1, recorded: -1, difference: -2, severity: 3,
                        description: Symbol::new(&env, "sign_flip_pos"),
                    };
                    out.push_back(m.clone());
                    Self::_push_mismatch(&env, m)?;
                }
                if neg_expected && rec.amount > 0 {
                    let m = MismatchRecord {
                        record_id: id, source: rec.source,
                        field: Symbol::new(&env, "sign"),
                        expected: -1, recorded: 1, difference: 2, severity: 3,
                        description: Symbol::new(&env, "sign_flip_neg"),
                    };
                    out.push_back(m.clone());
                    Self::_push_mismatch(&env, m)?;
                }
                // duplicate reference within same source (double-count)
                let pair = (rec.reference, rec.source as u32);
                let mut found = false;
                for p in seen.iter() { if p == pair { found = true; break; } }
                if found {
                    let m = MismatchRecord {
                        record_id: id, source: rec.source,
                        field: Symbol::new(&env, "reference"),
                        expected: 0, recorded: 1, difference: 1, severity: 3,
                        description: Symbol::new(&env, "double_count"),
                    };
                    out.push_back(m.clone());
                    Self::_push_mismatch(&env, m)?;
                } else {
                    seen.push_back(pair);
                }
            }
        }
        Ok(out)
    }

    pub fn get_mismatches(env: Env, start: u64, limit: u32) -> Vec<MismatchRecord> {
        let total: u64 = env.storage().persistent().get(&DataKey::MismatchCount).unwrap_or(0);
        let mut out = Vec::new(&env);
        let end = (start + limit as u64).min(total).saturating_add(1);
        for i in start..end {
            if let Some(m) = env.storage().persistent().get::<_, MismatchRecord>(
                &DataKey::Mismatch(i + 1)
            ) {
                out.push_back(m);
            }
        }
        out
    }

    /// -------------------------------------------------------------------
    /// 5. Historical transaction registry (audit trail)
    /// -------------------------------------------------------------------

    /// Register a historical transaction for replay validation.
    ///
    /// This powers the "historical transaction validation" acceptance
    /// criterion: anyone can replay a previously-recorded transaction
    /// through `replay_historical` to confirm it matches the current
    /// accounting state.
    pub fn register_historical(
        env:       Env,
        op:        Symbol,
        amount:    i128,
        token:     Address,
        sender:    Address,
        recipient: Address,
        reference: Symbol,
    ) -> Result<u64, Error> {
        Self::_assert_admin(&env)?;
        if amount == 0 { return Err(Error::InvalidAmount); }
        let count: u64 = env.storage().persistent().get(&DataKey::HistoricalCount).unwrap_or(0);
        let id = count.checked_add(1).ok_or(Error::Overflow)?;
        let tx = HistoricalTx {
            tx_id: id, op, amount,
            token: token.clone(),
            sender: sender.clone(),
            recipient: recipient.clone(),
            reference,
            timestamp: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&DataKey::Historical(id), &tx);
        env.storage().persistent().set(&DataKey::HistoricalCount, &id);
        Ok(id)
    }

    pub fn get_historical(env: Env, id: u64) -> Result<HistoricalTx, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Historical(id))
            .ok_or(Error::NoData)
    }

    pub fn get_historical_count(env: Env) -> u64 {
        env.storage().persistent().get(&DataKey::HistoricalCount).unwrap_or(0)
    }

    /// Replay the fee-math of a historical transaction against a known
    /// `fee_bps` — returns true iff recomputed_fee == tx.amount (for
    /// positive flow) or recomputed_net == -tx.amount (for negative).
    pub fn replay_historical_fee(
        env:     Env,
        tx_id:   u64,
        fee_bps: u32,
        gross_amount: i128,
    ) -> Result<bool, Error> {
        let tx = Self::get_historical(env.clone(), tx_id)?;
        let expected_fee = calc_platform_fee(gross_amount, fee_bps);
        let expected_net = gross_amount.saturating_sub(expected_fee);
        let ok = if tx.amount > 0 {
            // recorded as fee
            tx.amount == expected_fee
        } else {
            // recorded as net
            -tx.amount == expected_net
        };
        Ok(ok && expected_fee.saturating_add(expected_net) == gross_amount)
    }

    /// -------------------------------------------------------------------
    /// 6. Reconciliation report generation
    /// -------------------------------------------------------------------

    /// Run the full reconciliation pipeline:
    ///   (a) validate all fee distributions,
    ///   (b) validate referral rewards (with the supplied caps),
    ///   (c) check treasury balance consistency,
    ///   (d) run structural mismatch detection,
    ///   (e) aggregate into a `ReconciliationReport` and store it.
    pub fn generate_report(
        env:                  Env,
        start:                u64,
        end:                  u64,
        token:                Address,
        start_treasury_bal:   i128,
        max_multiplier_bps:   u32,
        max_lifetime_per_ref: i128,
        global_ref_cap:       i128,
        insurance_pool_bal:   i128,
        insurance_providers:  Vec<Address>,
    ) -> Result<ReconciliationReport, Error> {
        Self::_assert_admin(&env)?;
        if start > end { return Err(Error::InvalidRange); }

        let gen_ts = env.ledger().timestamp();

        // Reset per-run mismatch log (accumulated inside validators)
        let before_mismatches: u64 =
            env.storage().persistent().get(&DataKey::MismatchCount).unwrap_or(0);

        // (a)
        let fees = Self::validate_fee_distributions(env.clone(), start, end)?;
        let fee_val = fees.len() as u32;
        let mut fee_fail = 0u32;
        for f in fees.iter() { if !f.fee_matches || !f.net_matches || !f.sum_matches { fee_fail += 1; } }

        // (b)
        let refs = Self::validate_referral_rewards(
            env.clone(), start, end,
            max_multiplier_bps, max_lifetime_per_ref, global_ref_cap,
        )?;
        let ref_val = refs.len() as u32;
        let mut ref_fail = 0u32;
        for r in refs.iter() {
            if !r.matches { ref_fail += 1; }
        }

        // (c) treasury
        let probe = Self::check_treasury_consistency(
            env.clone(), token.clone(),
            start_treasury_bal, start, end,
        )?;
        let ins_entry = Self::check_insurance_pool_consistency(
            env.clone(), insurance_pool_bal, insurance_providers,
        )?;
        let ins_val = 1u32;
        let ins_fail = if ins_entry.pool_match { 0u32 } else { 1u32 };
        let probes = 1u32;
        let probes_failed = if probe.consistent { 0u32 } else { 1u32 };

        // (d) structural mismatches over the range
        let structural = Self::detect_mismatches(env.clone(), start, end)?;

        // Totals over the range
        let count: u64 = env.storage().persistent().get(&DataKey::RecordCount).unwrap_or(0);
        let (mut ins, mut outs, mut processed) = (0i128, 0i128, 0u64);
        for id in start..=end.min(count) {
            if let Some(rec) = env.storage().persistent().get::<_, LedgerRecord>(&DataKey::Record(id)) {
                processed += 1;
                if rec.amount >= 0 {
                    ins = ins.saturating_add(rec.amount);
                } else {
                    outs = outs.saturating_sub(rec.amount);
                }
            }
        }
        let net = ins.saturating_sub(outs);

        // Mismatches total (from the per-run counter)
        let after_mismatches: u64 =
            env.storage().persistent().get(&DataKey::MismatchCount).unwrap_or(0);
        // NOTE: detect_mismatches() itself pushes to the mismatch log via
        // `_push_mismatch`, so `after_mismatches - before_mismatches` already
        // includes both validator-raised AND structural mismatches.
        let found = after_mismatches.saturating_sub(before_mismatches);
        // Count critical (severity >= 2) from structural
        let mut critical = 0u32;
        for m in structural.iter() { if m.severity >= 2 { critical += 1; } }
        critical += fee_fail + ref_fail + ins_fail + probes_failed;

        let reconciling =
            fee_fail == 0 && ref_fail == 0 && ins_fail == 0
                && probe.consistent && found == 0;
        let tag = if reconciling {
            Symbol::new(&env, "CLEAN")
        } else if critical == 0 {
            Symbol::new(&env, "WARN")
        } else {
            Symbol::new(&env, "FAIL")
        };

        let treasury_end = probe.actual_balance;
        let computed_delta = probe.computed_balance
            .checked_sub(start_treasury_bal).unwrap_or(0);

        let report_id: u64 =
            env.storage().persistent().get(&DataKey::ReportCount).unwrap_or(0) + 1;
        let report = ReconciliationReport {
            report_id,
            generated_at: gen_ts,
            range_start: start,
            range_end: end,
            records_processed: processed,
            total_inflows: ins,
            total_outflows: outs,
            net_flow: net,
            fee_validations: fee_val,
            fee_failures: fee_fail,
            referral_validations: ref_val,
            referral_failures: ref_fail,
            insurance_validations: ins_val,
            insurance_failures: ins_fail,
            consistency_probes: probes,
            probes_failed,
            mismatches_found: found,
            critical_mismatches: critical,
            treasury_balance_start: start_treasury_bal,
            treasury_balance_end: treasury_end,
            treasury_computed_delta: computed_delta,
            reconciled: reconciling,
            summary_tag: tag,
        };
        env.storage().persistent().set(&DataKey::Report(report_id), &report);
        env.storage().persistent().set(&DataKey::ReportCount, &report_id);

        env.events().publish(
            (symbol_short!("recon"), symbol_short!("report"), report_id),
            (processed, found, tag.clone()),
        );
        Ok(report)
    }

    pub fn get_report(env: Env, id: u64) -> Result<ReconciliationReport, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Report(id))
            .ok_or(Error::NoData)
    }

    pub fn get_report_count(env: Env) -> u64 {
        env.storage().persistent().get(&DataKey::ReportCount).unwrap_or(0)
    }

    /// -------------------------------------------------------------------
    /// 7. View helpers
    /// -------------------------------------------------------------------

    pub fn get_running_totals(env: Env, source: LedgerSource) -> (i128, i128) {
        env.storage()
            .persistent()
            .get(&DataKey::RunningTotals(source))
            .unwrap_or((0, 0))
    }

    pub fn get_referrer_agg(env: Env, referrer: Address) -> (i128, i128) {
        env.storage()
            .persistent()
            .get(&DataKey::ReferrerAgg(referrer))
            .unwrap_or((0, 0))
    }

    pub fn get_insurance_provider_shares(env: Env, provider: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::InsuranceAgg(provider))
            .unwrap_or(0)
    }

    /// -------------------------------------------------------------------
    /// Internal helpers
    /// -------------------------------------------------------------------

    fn _raise_mismatch(
        env:         &Env,
        record_id:   u64,
        source:      LedgerSource,
        field:       Symbol,
        expected:    i128,
        recorded:    i128,
        severity:    u32,
        description: Symbol,
    ) -> Result<(), Error> {
        let diff = recorded.checked_sub(expected).unwrap_or(0);
        let m = MismatchRecord {
            record_id, source, field, expected, recorded,
            difference: diff, severity, description,
        };
        Self::_push_mismatch(env, m)
    }

    fn _raise_mismatch_raw(
        env:         &Env,
        record_id:   u64,
        source:      LedgerSource,
        field:       Symbol,
        expected:    i128,
        recorded:    i128,
        severity:    u32,
        description: Symbol,
    ) -> Result<(), Error> {
        let diff = recorded.checked_sub(expected).unwrap_or(0);
        let m = MismatchRecord {
            record_id, source, field, expected, recorded,
            difference: diff, severity, description,
        };
        Self::_push_mismatch(env, m)
    }

    fn _push_mismatch(env: &Env, m: MismatchRecord) -> Result<(), Error> {
        let count: u64 = env.storage().persistent().get(&DataKey::MismatchCount).unwrap_or(0);
        let id = count.checked_add(1).ok_or(Error::Overflow)?;
        env.storage().persistent().set(&DataKey::Mismatch(id), &m);
        env.storage().persistent().set(&DataKey::MismatchCount, &id);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Pure arithmetic helpers (no contract state) — exported so they can be
// tested directly and reused by off-chain reconcilers.
// ---------------------------------------------------------------------------

/// Platform fee = gross * fee_bps / 10_000  (truncating toward zero,
/// matching the production contracts `checked_mul().checked_div()`).
pub fn calc_platform_fee(gross_amount: i128, fee_bps: u32) -> i128 {
    if gross_amount <= 0 || fee_bps == 0 { return 0; }
    gross_amount
        .checked_mul(fee_bps as i128)
        .and_then(|v| v.checked_div(BPS_DENOM))
        .unwrap_or(0)
}

/// Insurance yield = platform_fee * YIELD_BPS / 10_000.
pub fn calc_insurance_yield(platform_fee: i128) -> i128 {
    calc_platform_fee(platform_fee, INSURANCE_YIELD_BPS as u32)
}

/// Fee-based referral reward = platform_fee * reward_bps / 10_000.
pub fn calc_referral_fee_reward(platform_fee: i128, reward_bps: u32) -> i128 {
    calc_platform_fee(platform_fee, reward_bps)
}

/// Apply multiplier bps and clamp reward to the lower of
/// `remaining_lifetime` / `remaining_global`.
pub fn apply_referral_caps(
    base_reward:         i128,
    multiplier_bps:      u32,
    remaining_lifetime:  i128,
    remaining_global:    i128,
) -> i128 {
    let raw = base_reward
        .checked_mul(multiplier_bps.max(10_000) as i128)
        .and_then(|v| v.checked_div(BPS_DENOM))
        .unwrap_or(0);
    let after_lt = raw.min(remaining_lifetime.max(0));
    after_lt.min(remaining_global.max(0))
}

// ---------------------------------------------------------------------------
// Minimal BTreeMap<Address, i128> shim (Vec-backed; Soroban-safe).
// ---------------------------------------------------------------------------

struct BTreeMap_Address_i128<'a> {
    env: &'a Env,
    keys: Vec<Address>,
    vals: Vec<i128>,
}

impl<'a> BTreeMap_Address_i128<'a> {
    fn new(env: &'a Env) -> Self {
        Self { env, keys: Vec::new(env), vals: Vec::new(env) }
    }
    fn get(&self, k: &Address) -> Option<i128> {
        for (i, key) in self.keys.iter().enumerate() {
            if key == *k { return Some(self.vals.get(i as u32).unwrap_or(0)); }
        }
        None
    }
    fn insert(&mut self, k: Address, v: i128) {
        for (i, key) in self.keys.iter().enumerate() {
            if key == k {
                let idx = i as u32;
                self.vals.set(idx, v);
                return;
            }
        }
        self.keys.push_back(k);
        self.vals.push_back(v);
        let _ = self.env; // silence unused
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{vec, Env, Symbol};

    mod token {
        use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};
        #[contracttype]
        pub enum K { Balance(Address) }
        #[contract]
        pub struct MockToken;
        #[contractimpl]
        impl MockToken {
            pub fn mint(env: Env, to: Address, a: i128) {
                let k = K::Balance(to);
                let b: i128 = env.storage().persistent().get(&k).unwrap_or(0);
                env.storage().persistent().set(&k, &(b + a));
            }
            pub fn transfer(_e: Env, _f: Address, _t: Address, _a: i128) {}
            pub fn balance(env: Env, id: Address) -> i128 {
                env.storage().persistent().get(&K::Balance(id)).unwrap_or(0)
            }
        }
    }

    struct Fixture {
        env:        Env,
        admin:      Address,
        recon_id:   Address,
        token:      Address,
        treasury:   Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(1_000_000);
            let admin    = Address::generate(&env);
            let escrow   = Address::generate(&env);
            let treasury = Address::generate(&env);
            let referral = Address::generate(&env);
            let insur    = Address::generate(&env);
            let token    = env.register_contract(None, token::MockToken);
            let recon_id = env.register_contract(None, ReconciliationEngine);
            let c = ReconciliationEngineClient::new(&env, &recon_id);
            c.initialize(&admin, &escrow, &treasury, &referral, &insur);
            token::MockTokenClient::new(&env, &token).mint(&treasury, &0);
            Fixture { env, admin, recon_id, token, treasury }
        }
        fn client(&self) -> ReconciliationEngineClient<'_> {
            ReconciliationEngineClient::new(&self.env, &self.recon_id)
        }
    }

    // ======================================================================
    // UNIT TESTS — pure arithmetic
    // ======================================================================

    #[test]
    fn calc_fee_matches_contract_formula() {
        // gross * bps / 10000 — truncating integer divide
        assert_eq!(calc_platform_fee(1000, 500),           50);   // 5%
        assert_eq!(calc_platform_fee(1000, 1000),         100);  // 10%
        assert_eq!(calc_platform_fee(1,    500),           0);    // truncation
        assert_eq!(calc_platform_fee(0,    500),           0);    // zero gross
        assert_eq!(calc_platform_fee(-100, 500),           0);    // negative
        assert_eq!(calc_platform_fee(10_000, 0),           0);    // zero bps
        assert_eq!(calc_platform_fee(10_000, 10_000), 10_000);    // 100%
    }

    #[test]
    fn calc_fee_rounds_toward_zero_no_dust() {
        // For any amount, fee + net == amount (no dust created or lost)
        for gross in [1, 7, 99, 100, 12345, 1_000_000_000] {
            for bps in [0, 1, 50, 500, 1000, 5000, 10_000] {
                let fee = calc_platform_fee(gross, bps);
                let net = gross - fee;
                assert_eq!(fee + net, gross,
                    "dust: gross={} bps={} fee={} net={}", gross, bps, fee, net);
                assert!(fee >= 0);
                assert!(net >= 0);
                assert!(fee <= gross);
            }
        }
    }

    #[test]
    fn referral_caps_applied() {
        // 100 base, 2x multiplier, lifetime=50, global=80 → 50 (lifetime binds)
        assert_eq!(apply_referral_caps(100, 20_000, 50, 80), 50);
        // 100 base, 1x, lifetime=500, global=80 → 80 (global binds)
        assert_eq!(apply_referral_caps(100, 10_000, 500, 80), 80);
        // 100, 1.5x, no cap scarcity → 150
        assert_eq!(apply_referral_caps(100, 15_000, 1000, 1000), 150);
        // Zero reward cases
        assert_eq!(apply_referral_caps(0, 20_000, 1000, 1000), 0);
        assert_eq!(apply_referral_caps(100, 20_000, 0, 1000),   0);
    }

    // ======================================================================
    // UNIT TESTS — fee distribution validation
    // ======================================================================

    #[test]
    fn validate_fees_clean_dataset_zero_failures() {
        let f = Fixture::setup();
        // Populate with correct fee records at 5% bps
        for (i, gross) in [1000i128, 2000, 7777, 1_000_000].into_iter().enumerate() {
            let fee = calc_platform_fee(gross, 500);
            f.client().append_record(
                &f.admin,
                &LedgerSource::EscrowFee,
                &f.token,
                &fee,                    // amount = +ve fee
                &500u32,                 // aux_bps
                &gross,                  // gross_base
                &Address::generate(&f.env),
                &Symbol::new(&f.env, &std::format!("esc{}", i)),
            ).unwrap();
        }
        let val = f.client().validate_fee_distributions(&1, &4);
        assert_eq!(val.len(), 4);
        for v in val.iter() {
            assert!(v.fee_matches);
            assert!(v.net_matches);
            assert!(v.sum_matches);
        }
        // No mismatches logged for clean set
        assert_eq!(f.client().get_mismatches(&0, &100).len(), 0);
    }

    #[test]
    fn validate_fees_detects_tampered_record() {
        let f = Fixture::setup();
        let gross = 1000i128;
        let correct_fee = calc_platform_fee(gross, 500); // 50
        // Tamper: record fee = 60 instead of 50
        f.client().append_record(
            &f.admin, &LedgerSource::EscrowFee, &f.token,
            &60, &500u32, &gross,
            &Address::generate(&f.env),
            &Symbol::new(&f.env, "esc_tampered"),
        ).unwrap();
        let val = f.client().validate_fee_distributions(&1, &1);
        assert_eq!(val.len(), 1);
        let v = val.get(0).unwrap();
        assert_eq!(v.expected_fee, correct_fee);
        assert_eq!(v.recorded_fee, 60);
        assert!(!v.fee_matches);
        // At least one mismatch raised
        assert!(f.client().get_mismatches(&0, &100).len() >= 1);
    }

    // ======================================================================
    // UNIT TESTS — referral reward validation
    // ======================================================================

    #[test]
    fn referral_validation_mentor_2x_multiplier() {
        let f = Fixture::setup();
        let referrer = Address::generate(&f.env);
        // mentor referral: aux_bps >= 1 signals mentor, multiplier is
        // max(aux_bps,10_000).min(max). Use 20000 → 2x base 50MNT = 100MNT.
        let base = REFERRAL_REWARD_MENTOR;
        let expected = (base * 20_000) / 10_000;
        f.client().append_record(
            &f.admin, &LedgerSource::ReferralReward,
            &f.token, &expected, &20_000u32, &base,
            &referrer, &Symbol::new(&f.env, "ref1"),
        ).unwrap();
        let refs = f.client().validate_referral_rewards(
            &1, &1,
            &20_000u32, &(1_000_000 * 10_000_000i128),
            &(5_000_000 * 10_000_000i128),
        ).unwrap();
        assert_eq!(refs.len(), 1);
        let r = refs.get(0).unwrap();
        assert_eq!(r.referee_type, 0); // mentor
        assert!(r.matches);
        assert!(r.within_lifetime_cap);
        assert!(r.within_global_cap);
    }

    #[test]
    fn referral_validation_learner_base_amount() {
        let f = Fixture::setup();
        let referrer = Address::generate(&f.env);
        let expected = (REFERRAL_REWARD_LEARNER * 10_000) / 10_000;
        f.client().append_record(
            &f.admin, &LedgerSource::ReferralReward,
            &f.token, &expected, &0u32,
            &REFERRAL_REWARD_LEARNER,
            &referrer, &Symbol::new(&f.env, "ref2"),
        ).unwrap();
        let refs = f.client().validate_referral_rewards(
            &1, &1,
            &20_000u32, &i128::MAX, &i128::MAX,
        ).unwrap();
        assert_eq!(refs.len(), 1);
        let r = refs.get(0).unwrap();
        assert_eq!(r.referee_type, 1); // learner (aux_bps=0)
        assert!(r.matches);
    }

    // ======================================================================
    // UNIT TESTS — treasury consistency probe
    // ======================================================================

    #[test]
    fn treasury_consistency_clean_flow() {
        let f = Fixture::setup();
        // Mint 1000 tokens to treasury address
        token::MockTokenClient::new(&f.env, &f.token).mint(&f.treasury, &1000);
        // 3 escrow fees totalling 350
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &100, &500u32, &2000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "a")).unwrap();
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &150, &500u32, &3000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "b")).unwrap();
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &100, &500u32, &2000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "c")).unwrap();
        // Alloc 50 out
        f.client().append_record(&f.admin, &LedgerSource::TreasuryAlloc,
            &f.token, &-50, &0u32, &0,
            &Address::generate(&f.env), &Symbol::new(&f.env, "al1")).unwrap();

        // Actual balance = 1000 (set on treasury). Computed: start=650 +350-50 = 950
        // → We expect inconsistent unless actual == computed.  For the test
        // we set actual = 950 by minting 950-initial=0 more: we minted 1000
        // so the probe will flag a +50 difference.
        let probe = f.client().check_treasury_consistency(
            &f.token, &650, &1, &4,
        ).unwrap();
        assert_eq!(probe.computed_balance, 950);
        assert_eq!(probe.actual_balance, 1000);
        assert_eq!(probe.difference, 50);
        assert!(!probe.consistent); // mismatches flagged
    }

    // ======================================================================
    // UNIT TESTS — structural mismatch detection
    // ======================================================================

    #[test]
    fn detect_flags_bps_over_cap_and_sign_flip() {
        let f = Fixture::setup();
        // 1. bad bps (20 000 > 10 000)
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &50, &20_000u32, &1000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "bad_bps")).unwrap();
        // 2. sign-flip: claim should be negative but we record positive
        f.client().append_record(&f.admin, &LedgerSource::InsuranceClaim,
            &f.token, &100, &0u32, &0,
            &Address::generate(&f.env), &Symbol::new(&f.env, "bad_sign")).unwrap();
        // 3. double-count: same reference/source again
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &50, &500u32, &1000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "dup")).unwrap();
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &50, &500u32, &1000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "dup")).unwrap();

        let mm = f.client().detect_mismatches(&1, &4).unwrap();
        let types: std::vec::Vec<&str> = mm.iter()
            .map(|m| {
                let sym = m.description;
                // We can't decode Symbol → str directly but count markers via severity
                match m.severity {
                    2 => "bps_or_critical",
                    3 => "critical",
                    _ => "warn",
                }
            })
            .collect();
        assert!(mm.len() >= 4, "expected bps + sign + 2*dup; got {:?}", types);
    }

    // ======================================================================
    // INTEGRATION TEST — historical tx validation
    // ======================================================================

    #[test]
    fn historical_tx_registration_and_replay() {
        let f = Fixture::setup();
        let sender = Address::generate(&f.env);
        let recipient = Address::generate(&f.env);
        // Register a 5% fee release on gross=1000
        let gross = 1000i128;
        let fee = calc_platform_fee(gross, 500);
        let id = f.client().register_historical(
            &Symbol::new(&f.env, "release_funds"),
            &fee,                    // tx.amount = fee portion (positive)
            &f.token,
            &sender,
            &recipient,
            &Symbol::new(&f.env, "esc123"),
        ).unwrap();
        assert_eq!(id, 1);
        assert_eq!(f.client().get_historical_count(), 1);

        // Replay: correct bps matches
        assert!(f.client().replay_historical_fee(&1, &500u32, &gross).unwrap());
        // Wrong bps fails
        assert!(!f.client().replay_historical_fee(&1, &1000u32, &gross).unwrap());
        // Wrong gross also fails
        assert!(!f.client().replay_historical_fee(&1, &500u32, &2000).unwrap());

        // Check stored fields
        let tx = f.client().get_historical(&1).unwrap();
        assert_eq!(tx.op, Symbol::new(&f.env, "release_funds"));
        assert_eq!(tx.amount, fee);
        assert_eq!(tx.reference, Symbol::new(&f.env, "esc123"));
    }

    #[test]
    fn historical_zero_amount_rejected() {
        let f = Fixture::setup();
        let r = f.client().try_register_historical(
            &Symbol::new(&f.env, "op"),
            &0, &f.token,
            &Address::generate(&f.env),
            &Address::generate(&f.env),
            &Symbol::new(&f.env, "ref"),
        );
        assert!(r.is_err());
    }

    // ======================================================================
    // INTEGRATION TEST — end-to-end report generation with a clean dataset
    // ======================================================================

    #[test]
    fn report_clean_dataset_matches_summary_tag_clean() {
        let f = Fixture::setup();
        token::MockTokenClient::new(&f.env, &f.token).mint(&f.treasury, &300);

        // --- Build a PERFECT dataset ---
        // 2 escrow fees (5%), total fee = 50 + 250 = 300
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &50,  &500u32, &1000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "e1")).unwrap();
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &250, &500u32, &5000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "e2")).unwrap();
        // 1 learner referral (2x multiplier = 40 MNT equivalent; we treat
        // base=20MNT, multiplier=20000bps, record=40)
        let ref_expected = (REFERRAL_REWARD_LEARNER * 20_000) / 10_000;
        let referrer = Address::generate(&f.env);
        f.client().append_record(&f.admin, &LedgerSource::ReferralReward,
            &f.token, &ref_expected, &0u32, &REFERRAL_REWARD_LEARNER,
            &referrer, &Symbol::new(&f.env, "r1")).unwrap();
        // Correct sign: 1 insurance claim -50
        let provider = Address::generate(&f.env);
        f.client().append_record(&f.admin, &LedgerSource::InsuranceClaim,
            &f.token, &-50, &0u32, &0,
            &provider, &Symbol::new(&f.env, "c1")).unwrap();

        // Set up insurance agg to match — pool_bal = 1_000, 2 providers with shares summing to 1_000
        let p2 = Address::generate(&f.env);
        f.client().append_record(&f.admin, &LedgerSource::InsuranceYield,
            &f.token, &400, &10u32, &0,
            &provider, &Symbol::new(&f.env, "y1")).unwrap();
        f.client().append_record(&f.admin, &LedgerSource::InsuranceYield,
            &f.token, &600, &10u32, &0,
            &p2,       &Symbol::new(&f.env, "y2")).unwrap();

        // Treasury actual = 300; start bal = 0; Σinflows = 50+250+ref (positive counted as inflow)
        // EscrowFee + ReferralReward are positive → inflows
        // Our dataset: actual=300, computed=0+50+250=300 for treasury-relevant sources
        let report = f.client().generate_report(
            &1, &6,
            &f.token,
            &0,                      // start treasury bal
            &20_000u32,               // max multiplier
            &i128::MAX,               // lifetime cap (unlimited)
            &i128::MAX,               // global cap (unlimited)
            &1000,                    // insurance pool = sum of shares (400+600)
            &vec![&f.env, provider.clone(), p2.clone()],
        ).unwrap();

        assert_eq!(report.fee_validations, 2);
        assert_eq!(report.fee_failures, 0);
        assert_eq!(report.referral_validations, 1);
        assert_eq!(report.referral_failures, 0);
        assert_eq!(report.insurance_validations, 1);
        assert_eq!(report.insurance_failures, 0);
        assert_eq!(report.probes_failed, 0);
        assert_eq!(report.critical_mismatches, 0);
        assert!(report.reconciled);
        assert_eq!(report.summary_tag, Symbol::new(&f.env, "CLEAN"));
        assert_eq!(report.report_id, 1);
        assert_eq!(f.client().get_report_count(), 1);
    }

    #[test]
    fn report_fail_dataset_flagged_fail_tag() {
        let f = Fixture::setup();
        // Tamper: bps over cap (15 000)
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &100, &15_000u32, &1000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "bad")).unwrap();
        token::MockTokenClient::new(&f.env, &f.token).mint(&f.treasury, &0);

        let report = f.client().generate_report(
            &1, &1, &f.token,
            &0, &20_000u32, &i128::MAX, &i128::MAX,
            &0, &vec![&f.env],
        ).unwrap();
        assert!(!report.reconciled);
        assert_eq!(report.summary_tag, Symbol::new(&f.env, "FAIL"));
        assert!(report.mismatches_found > 0);
    }

    // ======================================================================
    // Initialisation guards
    // ======================================================================

    #[test]
    fn double_init_rejected() {
        let f = Fixture::setup();
        let r = f.client().try_initialize(
            &f.admin,
            &Address::generate(&f.env), &Address::generate(&f.env),
            &Address::generate(&f.env), &Address::generate(&f.env),
        );
        assert!(r.is_err());
    }

    #[test]
    fn append_record_invalid_range_rejected() {
        let f = Fixture::setup();
        let r = f.client().try_validate_fee_distributions(&10, &1);
        assert!(r.is_err());
    }

    // ======================================================================
    // Running totals + account aggregates
    // ======================================================================

    #[test]
    fn running_totals_and_aggs_accumulate() {
        let f = Fixture::setup();
        let referrer = Address::generate(&f.env);
        let provider = Address::generate(&f.env);
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &100, &500u32, &2000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "a")).unwrap();
        f.client().append_record(&f.admin, &LedgerSource::EscrowFee,
            &f.token, &200, &500u32, &4000,
            &Address::generate(&f.env), &Symbol::new(&f.env, "b")).unwrap();
        f.client().append_record(&f.admin, &LedgerSource::ReferralReward,
            &f.token, &30, &0u32, &REFERRAL_REWARD_LEARNER,
            &referrer, &Symbol::new(&f.env, "r1")).unwrap();
        f.client().append_record(&f.admin, &LedgerSource::InsuranceYield,
            &f.token, &500, &10u32, &0,
            &provider, &Symbol::new(&f.env, "y")).unwrap();

        let (fi, fo) = f.client().get_running_totals(LedgerSource::EscrowFee);
        assert_eq!(fi, 300);
        assert_eq!(fo, 0);
        let (_, credited) = f.client().get_referrer_agg(referrer);
        assert_eq!(credited, 30);
        assert_eq!(f.client().get_insurance_provider_shares(provider), 500);
    }

    // ======================================================================
    // Amount invariants — zero, negative, max
    // ======================================================================

    #[test]
    fn calc_fee_never_panics_on_extreme_inputs() {
        for gross in [
            0, 1, -1, i128::MIN, i128::MAX,
            1_000_000_000_000_000i128,
            -999_999_999_999i128,
        ] {
            for bps in [0u32, 1, 500, 1_000, 5_000, 10_000, u32::MAX] {
                // Must never panic
                let fee = calc_platform_fee(gross, bps);
                // Safety: for positive gross & valid bps, fee never exceeds gross
                if gross >= 0 && bps <= 10_000 {
                    assert!(fee <= gross,
                        "fee={} > gross={} bps={}", fee, gross, bps);
                    assert!(fee >= 0);
                }
            }
        }
    }

    #[test]
    fn insurance_yield_matches_10bps() {
        // 10 bps = 0.1%
        assert_eq!(calc_insurance_yield(100_000), 10);
        assert_eq!(calc_insurance_yield(0), 0);
        assert_eq!(calc_insurance_yield(99), 0); // truncated
    }
}
