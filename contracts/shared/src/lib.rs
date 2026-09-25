#![no_std]

pub mod pagination;
pub mod pause_guard;
pub mod privacy;
pub mod state_machine;

pub use pagination::{Pagination, MAX_PAGE_SIZE};
pub use pause_guard::{
    is_paused, pause, pause_guardian, pause_state, require_not_paused, require_pause_guardian,
    set_pause_guardian, set_paused, unpause, PauseState,
};
pub use privacy::{
    contain_data_breach, detect_cross_session_leak, leak_log, record_session_owner, session_owner,
    CrossSessionLeakResult, LeakLogEntry,
};
pub use state_machine::StateMachine;

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    // ── pause_guard ──────────────────────────────────────────────────────────

    #[test]
    fn pause_state_defaults_to_active() {
        let env = Env::default();
        assert_eq!(pause_state(&env), PauseState::Active);
        assert!(!is_paused(&env));
    }

    #[test]
    fn pause_and_unpause_toggle_the_flag() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        set_pause_guardian(&env, &admin, &guardian);
        assert_eq!(pause_guardian(&env), Some(guardian.clone()));

        pause(&env);
        assert!(is_paused(&env));
        require_pause_guardian(&env);

        unpause(&env);
        assert!(!is_paused(&env));
    }

    #[test]
    fn set_paused_writes_the_flag_without_auth() {
        let env = Env::default();
        set_paused(&env, true);
        assert!(is_paused(&env));
        set_paused(&env, false);
        assert!(!is_paused(&env));
    }

    #[test]
    #[should_panic(expected = "contract paused")]
    fn require_not_paused_panics_while_paused() {
        let env = Env::default();
        set_paused(&env, true);
        require_not_paused(&env);
    }

    #[test]
    fn require_not_paused_passes_while_active() {
        let env = Env::default();
        require_not_paused(&env);
    }

    // ── pagination ───────────────────────────────────────────────────────────

    #[test]
    fn bounds_clamps_limit_to_max_page_size() {
        assert_eq!(Pagination::bounds(500, 0, 10_000), (0, MAX_PAGE_SIZE));
    }

    #[test]
    fn bounds_returns_full_range_when_limit_exceeds_total() {
        assert_eq!(Pagination::bounds(10, 0, 50), (0, 10));
    }

    #[test]
    fn bounds_honours_offset() {
        assert_eq!(Pagination::bounds(10, 6, 3), (6, 9));
    }

    #[test]
    fn bounds_returns_empty_page_past_the_end() {
        assert_eq!(Pagination::bounds(10, 10, 5), (10, 10));
        assert_eq!(Pagination::bounds(10, 99, 5), (10, 10));
    }

    #[test]
    fn bounds_handles_empty_collection() {
        assert_eq!(Pagination::bounds(0, 0, 5), (0, 0));
    }

    #[test]
    fn clamp_limit_never_yields_a_full_unbounded_page() {
        assert_eq!(Pagination::clamp_limit(0), 1);
        assert_eq!(Pagination::clamp_limit(1), 1);
        assert_eq!(Pagination::clamp_limit(MAX_PAGE_SIZE), MAX_PAGE_SIZE);
        assert_eq!(Pagination::clamp_limit(MAX_PAGE_SIZE + 1), MAX_PAGE_SIZE);
    }

    // ── privacy ──────────────────────────────────────────────────────────────

    #[test]
    fn session_owner_is_recorded_and_read_back() {
        let env = Env::default();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);

        record_session_owner(&env, &learner, &mentor);
        assert_eq!(session_owner(&env, &learner), Some(mentor));
    }

    #[test]
    fn owner_read_is_not_a_leak() {
        let env = Env::default();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        record_session_owner(&env, &learner, &mentor);

        let result = detect_cross_session_leak(&env, &mentor, &learner);
        assert_eq!(result, CrossSessionLeakResult::NoLeak);
        assert_eq!(result.severity(), 0);
    }

    #[test]
    fn learner_self_read_is_not_a_leak() {
        let env = Env::default();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        record_session_owner(&env, &learner, &mentor);

        assert_eq!(
            detect_cross_session_leak(&env, &learner, &learner),
            CrossSessionLeakResult::NoLeak
        );
    }

    #[test]
    fn foreign_mentor_read_is_logged_as_a_leak() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let accessor = Address::generate(&env);
        let learner = Address::generate(&env);
        record_session_owner(&env, &learner, &owner);

        let result = detect_cross_session_leak(&env, &accessor, &learner);
        assert!(result.is_leak());
        assert_eq!(result.severity(), 1);
        assert_eq!(
            result,
            CrossSessionLeakResult::Leak(owner, accessor.clone(), 0)
        );
    }

    #[test]
    fn repeat_foreign_reads_escalate_severity() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let accessor = Address::generate(&env);
        let learner = Address::generate(&env);
        record_session_owner(&env, &learner, &owner);

        assert_eq!(
            detect_cross_session_leak(&env, &accessor, &learner).severity(),
            1
        );
        assert_eq!(
            detect_cross_session_leak(&env, &accessor, &learner).severity(),
            2
        );
        assert_eq!(
            detect_cross_session_leak(&env, &accessor, &learner).severity(),
            3
        );

        let log = leak_log(&env);
        assert_eq!(log.len(), 1);
        assert_eq!(log.get(0).unwrap().hits, 3);
    }

    #[test]
    fn contain_data_breach_lists_distinct_offenders() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let learner = Address::generate(&env);
        let first = Address::generate(&env);
        let second = Address::generate(&env);
        record_session_owner(&env, &learner, &owner);

        detect_cross_session_leak(&env, &first, &learner);
        detect_cross_session_leak(&env, &first, &learner);
        detect_cross_session_leak(&env, &second, &learner);

        let offenders = contain_data_breach(&env, &learner);
        assert_eq!(offenders.len(), 2);
        assert!(offenders.contains(&first));
        assert!(offenders.contains(&second));
    }

    #[test]
    fn breach_report_is_scoped_per_learner() {
        let env = Env::default();
        let learner_a = Address::generate(&env);
        let learner_b = Address::generate(&env);
        let accessor = Address::generate(&env);

        detect_cross_session_leak(&env, &accessor, &learner_a);

        assert_eq!(contain_data_breach(&env, &learner_a).len(), 1);
        assert_eq!(contain_data_breach(&env, &learner_b).len(), 0);
    }
}
