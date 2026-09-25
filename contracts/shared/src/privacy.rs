use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

/// Instance storage key for the append-only cross-mentor access log.
///
/// Symbol-keyed because `shared` cannot know the host contract's `DataKey`.
/// Storage is namespaced per contract instance, so the key is collision-free.
const LEAK_LOG: Symbol = symbol_short!("LEAK_LOG");

/// Instance storage key for the per-learner session owner.
///
/// A learner can hold sessions with several mentors, so ownership is recorded
/// per learner and refreshed by the host contract on every session write.
const LEAK_OWNER: Symbol = symbol_short!("LEAK_OWNR");

/// Outcome of an advisory cross-session access check.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CrossSessionLeakResult {
    /// The accessor is the learner, or a mentor who owns the session. Nothing
    /// to report.
    NoLeak,
    /// A mentor read a learner's session data without owning it:
    /// `(session owner, accessor, prior foreign accesses for this pair)`.
    Leak(Address, Address, u32),
}

impl CrossSessionLeakResult {
    pub fn is_leak(&self) -> bool {
        matches!(self, CrossSessionLeakResult::Leak(..))
    }

    /// 1 for a first-offence alert, higher for repeat access by the same
    /// mentor. Governance can escalate on the severity.
    pub fn severity(&self) -> u32 {
        match self {
            CrossSessionLeakResult::NoLeak => 0,
            CrossSessionLeakResult::Leak(_, _, prior_foreign_accesses) => {
                prior_foreign_accesses + 1
            }
        }
    }
}

/// One row per `(learner, accessor)` pair, with a hit counter, so the log grows
/// with distinct offenders rather than with request volume.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeakLogEntry {
    pub learner: Address,
    pub accessor: Address,
    pub hits: u32,
    pub last_seen: u64,
}

/// Records which mentor a learner's session data belongs to.
///
/// Called by the host contract whenever a session is registered or transferred,
/// so [`detect_cross_session_leak`] can tell a legitimate mentor read from a
/// cross-mentor one.
pub fn record_session_owner(env: &Env, learner: &Address, mentor: &Address) {
    env.storage()
        .persistent()
        .set(&(LEAK_OWNER, learner.clone()), mentor);
}

/// Advisory cross-session privacy check.
///
/// Takes the accessing mentor and the learner whose data is being read. When
/// the accessor is neither the learner nor the learner's session owner, the
/// access is logged as a cross-mentor leak pattern.
///
/// The read itself is never blocked: mentors legitimately audit learner history
/// and breaking that would be a regression. Detection only records the access
/// and surfaces it to governance.
pub fn detect_cross_session_leak(
    env: &Env,
    accessor: &Address,
    learner: &Address,
) -> CrossSessionLeakResult {
    if accessor == learner {
        return CrossSessionLeakResult::NoLeak;
    }

    let owner = session_owner(env, learner);

    if let Some(ref recorded) = owner {
        if accessor == recorded {
            return CrossSessionLeakResult::NoLeak;
        }
    }

    let prior_foreign_accesses = record_foreign_access(env, accessor, learner);

    CrossSessionLeakResult::Leak(
        owner.unwrap_or_else(|| accessor.clone()),
        accessor.clone(),
        prior_foreign_accesses,
    )
}

pub fn session_owner(env: &Env, learner: &Address) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&(LEAK_OWNER, learner.clone()))
}

/// Returns every distinct mentor that read a learner's session data without
/// owning it, so governance can revoke access and notify the learner.
pub fn contain_data_breach(env: &Env, learner: &Address) -> Vec<Address> {
    let log: Vec<LeakLogEntry> = log_entries(env);

    let mut offenders = Vec::new(env);
    for entry in log.iter() {
        if entry.learner == *learner && !offenders.contains(&entry.accessor) {
            offenders.push_back(entry.accessor);
        }
    }
    offenders
}

/// Full log, for off-chain incident review.
pub fn leak_log(env: &Env) -> Vec<LeakLogEntry> {
    log_entries(env)
}

fn log_entries(env: &Env) -> Vec<LeakLogEntry> {
    env.storage()
        .persistent()
        .get(&LEAK_LOG)
        .unwrap_or_else(|| Vec::new(env))
}

fn record_foreign_access(env: &Env, accessor: &Address, learner: &Address) -> u32 {
    let mut log = log_entries(env);
    let now = env.ledger().timestamp();
    let mut prior = 0u32;

    for i in 0..log.len() {
        let entry = log.get(i).unwrap();
        if entry.learner == *learner && entry.accessor == *accessor {
            prior = entry.hits;
            let mut bumped = entry.clone();
            bumped.hits += 1;
            bumped.last_seen = now;
            log.set(i, bumped);
            env.storage().persistent().set(&LEAK_LOG, &log);
            return prior;
        }
    }

    log.push_back(LeakLogEntry {
        learner: learner.clone(),
        accessor: accessor.clone(),
        hits: 1,
        last_seen: now,
    });
    env.storage().persistent().set(&LEAK_LOG, &log);
    prior
}
