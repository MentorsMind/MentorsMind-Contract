//! Shared staking primitives (cross-crate shared between `staking` and `snapshot` contracts).
//!
//! Source-of-truth definition of `StakeRecord` lives here. Both crates MUST
//! import it from this shared crate rather than redefining locally, because
//! Soroban serializes `#[contracttype]` structs positionally by field-order
//! in XDR. Any local re-definition that diverges in field count, field order,
//! or field type will silently produce corrupted values on `from_val` — the
//! exact class of bug this module was extracted to fix (see GitHub issue
//! #646).
//!
//!     StakeRecord
//!     ============
//!     Field order is PART OF THE WIRE FORMAT and MUST NOT CHANGE without a
//!     coordinated redeployment of BOTH contracts together with a migration.

use soroban_sdk::{contracttype, Address};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeRecord {
    pub mentor: Address,
    pub amount: i128,
    pub staked_at: u64,
    pub unlock_at: u64,
    pub unlock_cooldown_until: Option<u64>,
    /// Tier of the mentor: 0 = None, 1 = Bronze, 2 = Silver, 3 = Gold.
    ///
    /// Stored as `u32` for alignment with governance/escrow tiers and
    /// future tier enums. The previous snapshot contract originally declared
    /// this as `u8` inside a loop body — that created a silent positional XDR
    /// mismatch whenever tier read the Option discriminant bytes instead and
    /// produced tier = 0 even for Gold mentors.
    pub tier: u32,
}

/// Companion event payload matching the StakeRecord shape, also shared so
/// the staking→governance event consumers reuse the same definition.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakedEventData {
    pub mentor: Address,
    pub amount: i128,
    pub unlock_at: u64,
    pub unlock_cooldown_until: Option<u64>,
    /// Matches `StakeRecord.tier`
    pub tier: u32,
}
