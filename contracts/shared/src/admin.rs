use soroban_sdk::{contracttype, Address};

/// Represents the state of an admin change proposal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdminChangeProposal {
    Proposed,
    Accepted,
    Revoked,
}

/// Stores information about a pending admin transfer.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransfer {
    pub new_admin: Address,
    pub effective_at: u64,
    pub status: AdminChangeProposal,
}

/// The minimum timelock delay required before a new admin can accept the role.
pub const MIN_ADMIN_TIMELOCK_SECS: u64 = 48 * 60 * 60; // 48 hours

/// A cooling-off period required between consecutive admin changes.
pub const ADMIN_COOLING_OFF_SECS: u64 = 24 * 60 * 60; // 24 hours
