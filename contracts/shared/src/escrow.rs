use soroban_sdk::{contracttype, Address, Symbol, Env};
use crate::state_machine::StateMachine;

/// Shared escrow status enum used across escrow and reputation contracts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    /// Escrow created but funds not yet deposited (pre-funding state).
    Pending,
    Active,
    Released,
    Disputed,
    Refunded,
    /// Dispute was resolved by admin arbitration via `resolve_dispute`.
    Resolved,
}

impl StateMachine for EscrowStatus {
    type State = Self;

    fn is_valid_transition(_env: &Env, from: &Self::State, to: &Self::State) -> bool {
        matches!(
            (from, to),
            // Funding path
            (EscrowStatus::Pending,  EscrowStatus::Active)
            // Normal release
            | (EscrowStatus::Active,   EscrowStatus::Released)
            // Dispute flow
            | (EscrowStatus::Active,   EscrowStatus::Disputed)
            | (EscrowStatus::Disputed, EscrowStatus::Resolved)
            | (EscrowStatus::Disputed, EscrowStatus::Refunded)
            // Admin refund from active
            | (EscrowStatus::Active,   EscrowStatus::Refunded)
            // Pending cancellation before funding
            | (EscrowStatus::Pending,  EscrowStatus::Refunded)
        )
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowTransitionLog {
    pub from: EscrowStatus,
    pub to: EscrowStatus,
    pub actor: Address,
    pub timestamp: u64,
}

/// Shared escrow record structure used across escrow and reputation contracts.
/// This ensures a single source of truth for the escrow data structure.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EscrowRecord {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    /// Platform fee deducted at release time (0 until released).
    pub platform_fee: i128,
    /// Amount actually sent to mentor after fee (0 until released).
    pub net_amount: i128,
    /// Unix timestamp (seconds) at which the session ends.
    pub session_end_time: u64,
    /// Seconds after `session_end_time` before auto-release may trigger.
    pub auto_release_delay: u64,
    /// Reason symbol provided when a dispute was opened (default: empty symbol).
    pub dispute_reason: Symbol,
    /// Unix timestamp (seconds) at which `resolve_dispute` was called (0 until resolved).
    pub resolved_at: u64,
    pub usd_amount: i128,
    pub quoted_token_amount: i128,
    pub send_asset: Address,
    pub dest_asset: Address,
    pub total_sessions: u32,
    pub sessions_completed: u32,
}
