use soroban_sdk::{contracttype, BytesN, Symbol};

/// Maximum number of continuity backups retained per session.
pub const MAX_BACKUP_RECORDS: u32 = 10;

/// Maximum age (seconds) before a backup is considered stale.
pub const BACKUP_MAX_AGE_SECS: u64 = 86_400;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuityBackup {
    pub session_id: Symbol,
    pub snapshot_at: u64,
    pub state_hash: BytesN<32>,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuityStatus {
    pub session_id: Symbol,
    pub has_backup: bool,
    pub latest_backup_at: u64,
    pub backup_active: bool,
}

/// Determine whether a session needs a new backup snapshot based on time
/// elapsed since the last one.
pub fn needs_backup(last_backup_at: u64, now: u64) -> bool {
    if last_backup_at == 0 {
        return true;
    }
    now.saturating_sub(last_backup_at) >= BACKUP_MAX_AGE_SECS
}

/// Validate that a backup record is still fresh and usable.
pub fn is_backup_valid(backup: &ContinuityBackup, now: u64) -> bool {
    backup.active && now.saturating_sub(backup.snapshot_at) < BACKUP_MAX_AGE_SECS
}
