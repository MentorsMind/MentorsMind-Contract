//! TTL utilities and heuristics for bumping persisted entries.
use soroban_sdk::Env;

/// Suggests a next bump interval (in seconds) based on the current TTL.
/// This is a lightweight heuristic: larger TTLs get proportionally larger intervals.
pub fn next_bump_interval(_env: &Env, current_ttl_secs: u64) -> u64 {
    if current_ttl_secs >= 86_400 {
        // If TTL >= 1 day, bump at half the TTL up to once per day.
        core::cmp::min(current_ttl_secs / 2, 86_400)
    } else if current_ttl_secs >= 3_600 {
        // Hourly-range TTLs: bump every 30 minutes.
        core::cmp::min(current_ttl_secs / 2, 1_800)
    } else {
        // Short TTLs: bump moderately frequently.
        core::cmp::max(60, current_ttl_secs / 4)
    }
}

/// Decide whether to bump TTL now given the current remaining TTL and time
/// since last bump. This function encodes a cost/benefit heuristic: bump when
/// the remaining TTL is less than a fraction of the desired persistence window
/// or when the time since last bump exceeds a fraction of that window.
pub fn should_bump_ttl(
    _env: &Env,
    remaining_ttl_secs: u64,
    time_since_last_bump_secs: u64,
    desired_persist_secs: u64,
) -> bool {
    if desired_persist_secs == 0 {
        return false;
    }

    // If remaining TTL is below 25% of desired persistence, bump now.
    if remaining_ttl_secs * 4 <= desired_persist_secs {
        return true;
    }

    // If we haven't bumped for more than 50% of desired persistence, bump now.
    if time_since_last_bump_secs * 2 >= desired_persist_secs {
        return true;
    }

    false
}
