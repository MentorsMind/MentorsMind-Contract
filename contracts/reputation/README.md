# Reputation Contract

The reputation contract (crate `mentorminds-reputation`) is a Soroban contract for the MentorMinds platform. It tracks bidirectional mentor/learner reviews for escrow-released sessions, detects fraud and collusion, scores predatory mentor conduct and learner welfare, monitors mentor burnout and market health, and exposes slash-penalty-adjusted reputation ratings. Fraud/welfare analytics are computed by pure functions and types from the sibling `shared` crate.

## Overview

- Bidirectional reviews: learners rate mentors (`submit_review`) and mentors rate learners (`submit_learner_review`); both require the session to be `Released` in the escrow contract.
- Review security: optional stake collection via a configured review token, completion-proof validation against a session registry, and per-review quality reports.
- Fraud detection: coordination flags, social-proof gaming, outcome-authenticity bursts, success-metric gaming, assessment-criteria coordination, grade inflation, and disinformation audits.
- Learner protection: predatory-behavior scoring, pair-level exploitation patterns, welfare status, and cooldown-gated interventions restorable by an arbitrator.
- Mentor conduct and wellness: compounding slash-penalty BPS with rehabilitation, workload/burnout tracking, and automatic emergency-pause interventions at critical risk.
- Ratings are running sums/counts (mentor and learner), with loyalty tiers, threshold proofs, and cross-platform reputation bridging.

## Entry Points

### Initialization / Config

- `fn initialize(escrow_contract: Address)` — Stores the escrow contract address; panics if already initialized.
- `fn configure_review_security(admin: Address, session_registry: Address, review_token: Address, base_stake: i128)` — Escrow-authority-only setup of session registry, review token, and base stake for review staking.
- `fn set_outcome_baseline(admin: Address, mentor: Address, baseline_bps: u32)` — Escrow-authority-only trusted baseline (0-10000 bps) for `protect_success_metrics`.

### Reviews & Ratings

- `fn submit_review(session_id: Symbol, mentor: Address, learner: Address, rating: u32, comment_hash: BytesN<32>)` — Learner-submitted 1-5 mentor review; verifies escrow `Released` status, collects stake, runs fraud/community/conduct monitoring, updates the mentor's rating average, emits an event.
- `fn submit_learner_review(mentor: Address, session_id: Symbol, learner: Address, participation_rating: u32, comment_hash: BytesN<32>)` — Mentor-submitted 1-5 learner participation review; verifies escrow status, updates the learner's rating average, emits an event.
- `fn update_reputation(user: Address, new_rating: u32)` — Appends a 1-5 rating to the user's mentor rating sum/count.
- `fn update_mentor_rating(mentor: Address, rating: u32)` — Appends a rating via the shared recovery wrapper.
- `fn migrate_legacy_rating(user: Address, legacy_rating: u32, legacy_count: u32)` — Seeds rating sum/count from legacy data.
- `fn file_review_dispute(mentor: Address, session_id: Symbol, reason_hash: BytesN<32>)` — Mentor files a dispute within the 14-day window; records a complaint signal.
- `fn resolve_review_dispute(arbitrator: Address, session_id: Symbol, remove_review: bool, adjusted_rating: Option<u32>)` — Arbitrator removes, adjusts, or rejects a pending dispute review and updates averages.
- `fn generate_threshold_proof(mentor: Address, min_rating: u32, secret_nonce: BytesN<32>) -> ReputationThresholdProof` — SHA-256 commitment proving the mentor's rating meets a threshold without revealing it.
- `fn verify_threshold_proof(proof: ReputationThresholdProof, actual_rating: u32, secret_nonce: BytesN<32>, verifier_challenge: Symbol) -> bool` — Recomputes and checks the commitment.
- `fn accrue_loyalty_points(user: Address, points: u32)` — Adds loyalty points and updates the tier (silver/gold/platinum thresholds).

### Collusion / Fraud

- `fn detect_collusion(env, mentor: Address, learner: Address) -> CollusionDetection` — Flags self-pairing (mentor == learner) as high coordination risk; stores a collusion signal.
- `fn verify_incentive_compatibility(mentor: Address) -> IncentiveCompatibilityResult` — Derives strategy-proofness from the stored collusion signal.
- `fn validate_community_interactions(mentor: Address, learner: Address) -> CoordinationFlag` — Scores pair-interaction timestamps for repeated/coordinated review timing.
- `fn monitor_social_proof_auth(mentor: Address) -> SocialProofRecord` — Scores a mentor's review history for endorsement bursts from few distinct reviewers.
- `fn get_community_status(mentor: Address) -> CommunityInterventionRecord` — Combines coordination and social-proof scores into an intervention decision.
- `fn restore_fair_participation(arbitrator: Address, mentor: Address)` — Arbitrator clears a community intervention after the cooldown.
- `fn authenticate_learning_outcomes(mentor: Address) -> OutcomeAuthenticity` — Treats narrow-evaluator rating bursts as manipulated outcome attestations.
- `fn protect_success_metrics(mentor: Address) -> SuccessMetricProtection` — Compares the mentor's rating-derived metric against the trusted baseline to detect gaming.
- `fn record_assessment_proposal(mentor: Address, proposer: Address)` — Logs an assessment-criteria proposal and distinct-proposer counts.
- `fn validate_assessment_criteria(mentor: Address) -> AssessmentValidation` — Detects a coordinated bloc setting evaluation standards.
- `fn get_outcome_status(mentor: Address) -> OutcomeInterventionRecord` — Combines outcome, metric, and assessment signals into an intervention decision.
- `fn restore_authentic_outcomes(arbitrator: Address, mentor: Address)` — Arbitrator clears an outcome-protection intervention after the cooldown.
- `fn record_grade_for_analysis(mentor: Address, session_id: Symbol, grade: u32)` — Appends a 0-10000 bps grade; runs inflation detection once enough history exists and applies a slash adjustment.
- `fn record_grade_correction(admin: Address, mentor: Address, learner: Address, session_id: Symbol, original_grade: u32, corrected_grade: u32, reason: Symbol) -> GradeCorrectionRecord` — Admin-authorized retroactive grade correction.
- `fn verify_session_information(session_id: Symbol, mentor: Address, claim_accuracy_bps: u32, source_authenticity_bps: u32) -> InformationIntegrity` — Per-session disinformation-risk check.
- `fn track_information_accuracy(mentor: Address, accurate_claims: u32, total_claims: u32, misinformation_signals: u32) -> InformationAuditRecord` — Cross-session accuracy audit and metadata-manipulation monitoring.
- `fn restore_reputation_truth(arbitrator: Address, mentor: Address) -> TruthRestorationRecord` — Arbitrator clears misinformation audit records after arbitration.
- `fn import_external_reputation(user: Address, platform: Symbol, external_score: u32) -> u32` — Stores an external score after applying an isolation discount (auth required).
- `fn validate_bridged_credentials(user: Address, credential_hash: BytesN<32>) -> bool` — Checks and records a bridged credential's validity flag.
- `fn assess_learning_outcomes(env, mentor: Address, outcome_score: u32) -> OutcomeAssessment` — Wraps an outcome score with incentive-alignment check.
- `fn track_curriculum_effectiveness(mentor: Address, metric: u32)` — Emits a curriculum-tracking event.

### Learner Welfare

- `fn identify_exploitation_patterns(mentor: Address, learner: Address, vulnerability: VulnerabilityAssessment) -> Vec<ExploitationPattern>` — Combines the mentor's predatory score with a vulnerability assessment; persists pair patterns.
- `fn track_learner_welfare(mentor: Address, learner: Address, vulnerability: VulnerabilityAssessment) -> LearnerProtectionRecord` — Computes welfare status from vulnerability plus active pattern count and produces an intervention record.
- `fn restore_learner_protection(arbitrator: Address, mentor: Address)` — Arbitrator clears a learner-protection intervention and predatory record after the cooldown.

### Mentor Conduct

- `fn track_mentor_conduct(mentor: Address, low_quality_session: bool, is_complaint: bool, price_above_market_bps: u32) -> PredatoryBehaviorDetection` — Updates complaint/consecutive-low-quality counters and recomputes the predatory score.
- `fn apply_slash_penalty(mentor: Address, slash_count: u32)` — Sets compounding slash-penalty BPS (500/1500/3000...) and clears rehabilitation.
- `fn rehabilitate_mentor(mentor: Address)` — Marks rehabilitation (halves the applied penalty in `get_mentor_rating`).

### Market / Workload Analytics

- `fn update_mentor_workload(mentor: Address, active_sessions: u32, weekly_hours: u32, weekly_weighted_load: u32, last_session_end: u64, rest_until: u64)` — Stores workload, computes burnout risk, and triggers an emergency-pause intervention at critical risk.
- `fn record_market_metrics(admin: Address, specialization: Symbol, total_sessions: u32, unique_mentors: u32, unique_learners: u32, avg_price: u64, median_price: u64, price_std_dev: u64, demand_index: u32, supply_index: u32, velocity: u32, concentration_ratio: u32)` — Admin-authorized weekly market metrics for a specialization.
- `fn assess_specialization_demand(specialization: Symbol, external_market_data: Map<Symbol, u64>) -> Option<DemandAuthenticityResult>` — Checks demand authenticity; may emit a market-manipulation alert.
- `fn correlate_quality_with_workload(mentor: Address) -> (u32, u32)` — Returns (quality score, workload score) pair.
- `fn track_mentor_wellness(mentor: Address) -> (u32, u32, Symbol)` — Returns (workload score, burnout score, risk level).
- `fn calculate_mentor_visibility(mentor: Address) -> u32` — Review-count-based visibility capped at 10000.
- `fn determine_recommendation_scores(mentor: Address) -> u32` — Gaming-resistant recommendation score capped at 10000.

### Views

- `fn get_mentor_rating(mentor: Address) -> (u64, u64)` — (average * 100, review count) after slash penalty (halved when rehabilitated).
- `fn get_learner_rating(learner: Address) -> (u64, u64)` — (average * 100, review count).
- `fn get_review(session_id: Symbol) -> ReviewRecord`; `fn get_learner_review(session_id: Symbol) -> LearnerReviewRecord` — Stored review records.
- `fn get_review_risk(mentor: Address) -> u32`; `fn get_review_quality_report(session_id: Symbol) -> ReviewQualityReport` — Cached risk score and per-review quality report.
- `fn calculate_average_rating(user: Address) -> u32` — Simple average * 100 without slash adjustment.
- `fn get_loyalty_points(user: Address) -> u32`; `fn get_loyalty_tier(user: Address) -> u32`; `fn get_loyalty_discount_bps(user: Address) -> u32` — Loyalty state (tier discounts: 0/500/1000/1500 bps).
- `fn get_grade_distribution(mentor: Address) -> GradeDistributionStats`; `fn get_inflation_detection(mentor: Address) -> Option<InflationDetectionResult>` — Grade-analysis views.
- `fn get_mentor_workload(mentor: Address) -> Option<MentorWorkload>`; `fn get_burnout_assessment(mentor: Address) -> Option<BurnoutRiskAssessment>`; `fn get_wellness_intervention(mentor: Address) -> Option<WellnessIntervention>` — Wellness views.
- `fn get_market_alert(alert_id: Symbol) -> Option<MarketManipulationAlert>`; `fn get_specialization_metrics(specialization: Symbol) -> Option<MarketMetrics>` — Market views.
- `fn calculate_reputation_score(mentor: Address) -> u32` — Raw score from sum/count without slash adjustment.

## Storage

| `DataKey` variant | Purpose |
| --- | --- |
| `NamespaceRoot` | Contract-isolated storage namespace root |
| `Review(Symbol)` / `LearnerReview(Symbol)` | Mentor / learner review records per session |
| `MentorRatingSum(Address)` / `MentorReviewCount(Address)` | Mentor rating running sum and count |
| `LearnerRatingSum(Address)` / `LearnerReviewCount(Address)` | Learner rating running sum and count |
| `LoyaltyPoints(Address)` / `LoyaltyTier(Address)` | Loyalty points and computed tier |
| `SlashPenaltyBps(Address)` / `Rehabilitated(Address)` | Slash penalty BPS and rehabilitation flag |
| `ReviewDispute(Symbol)` | Dispute record per session |
| `ThresholdProof(Address, u32)` | Stored rating-threshold proofs |
| `SessionRegistry` / `ReviewToken` / `ReviewStakeBase` | Configured registry address, stake token, base stake (instance storage) |
| `ReviewTimestamps(Address)` / `ReviewRatings(Address)` / `ReviewSignalScore(Address)` | Rolling 10-entry review history and risk score |
| `PairInteractionLog(Address, Address)` / `HasReviewedBefore(Address, Address)` / `DistinctReviewerCount(Address)` | Coordination and distinct-reviewer tracking |
| `CommunityCoordination(Address)` / `SocialProofScore(Address)` / `CommunityIntervention(Address)` | Community-dynamics scores and intervention record |
| `OutcomeAuthenticityRecord(Address)` / `MetricBaseline(Address)` / `SuccessMetricRecord(Address)` | Outcome-authenticity and metric-gaming state |
| `AssessmentProposalLog(Address)` / `AssessmentHasProposedBefore(Address, Address)` / `AssessmentDistinctProposerCount(Address)` / `AssessmentValidationRecord(Address)` | Assessment-criteria coordination tracking |
| `OutcomeIntervention(Address)` | Combined outcome-protection intervention record |
| `ExternalReputation(Address, Symbol)` / `BridgedCredentials(Address, BytesN<32>)` | Cross-platform reputation bridging |
| `MentorConductLog(Address)` / `MentorComplaintCount(Address)` / `MentorConsecutiveLowQuality(Address)` / `MentorPredatoryBehaviorRecord(Address)` | Conduct counters and predatory-behavior record |
| `MentorExploitationPatterns(Address, Address)` / `LearnerWelfareStatus(Address, Address)` / `LearnerProtectionIntervention(Address)` | Pair exploitation patterns and learner protection |
| `ReviewQualityReport(Symbol)` | Per-review quality/authenticity audit |
| `CollusionSignal(Address)` / `IncentiveCompatibility(Address)` | Collusion detection state |
| `GradeCorrection(Symbol)` / `GradeInflationDetection(Address)` / `MentorGradeHistory(Address)` / `MentorGradeTimestamps(Address)` | Grade-inflation analysis |
| `InformationAccuracyTrack(Address)` / `MisinformationDetection(Address)` / `ReputationInfoAudit(Address)` / `ReputationTruthRestoration(Address)` / `SessionInfoVerification(Symbol)` | Information-integrity tracking |
| `MarketManipulationAlert(Symbol)` / `SpecializationMetrics(Symbol)` | Market monitoring |
| `MentorBurnoutAssessment(Address)` / `MentorWorkloadData(Address)` / `WellnessIntervention(Address)` | Mentor wellness |

## Events

| Topic | Data | Emitted when |
| --- | --- | --- |
| `(review, review_submitted, mentor)` | session, learner, rating, timestamp | Mentor review submitted |
| `(lr_review, learner_review_submitted, learner)` | session, mentor, rating, timestamp | Learner review submitted |
| `(coord, flagged)` | mentor, risk score | Pair coordination flagged |
| `(sproof, flagged)` | mentor, gaming risk score | Social proof not genuine |
| `(commrest, restored)` | mentor | Community participation restored |
| `(outcome, flagged)` | mentor, manipulation risk score | Outcomes not genuine |
| `(metric, gaming)` | mentor, gaming risk score | Success-metric gaming detected |
| `(assess, flagged)` | mentor, coordination risk score | Assessment criteria not objective |
| `(conduct, predatory)` | mentor, risk score | Predatory behavior detected |
| `(exploit, detected)` | mentor, learner, pattern count | Exploitation patterns found |
| `(welfare, support)` | learner, mentor, welfare risk score | Support services activated |
| `(lprot, intervene)` | mentor, combined risk score | Learner-protection intervention |
| `(lprest, restored)` / `(outrest, restored)` | mentor | Learner-protection / outcome intervention restored |
| `(slash, pen)` / `(slash, rehab)` | mentor[, bps] | Slash penalty applied / mentor rehabilitated |
| `(Reput, updated)` | user, updated average | Direct reputation update |
| `("loyalty_points_accrued", user)` | total, tier | Loyalty points accrued |
| `("ReviewDisputeFiled", mentor)` / `("ReviewDisputeResolved", session)` | session / dispute status | Dispute filed / resolved |
| `(verify, disinfo)` | (session, mentor, risk score), risk score | Session disinformation flagged |
| `(track, misinfo)` | (mentor, score), score | Accuracy not verified |
| `(resttruth, restored)` | mentor, restored accuracy bps | Reputation truth restored |
| `(grade, inflation_detected)` / `(grade, corrected)` | mentor + rates / correction pair | Grade inflation detected / corrected |
| `(wellness, intervention_triggered)` | mentor, intervention type, duration | Critical burnout risk |
| `(market, manipulation_alert)` | specialization, type, severity | Market manipulation suspected |
| `(curric, tracked)` | mentor, metric | Curriculum effectiveness tracked |

## Cross-Contract Dependencies

- **Escrow contract** — Address stored by `initialize`; `submit_review` and `submit_learner_review` invoke `get_escrow_by_session` and require status `Released`. The escrow address is also the only authority for `configure_review_security` and `set_outcome_baseline`.
- **Session registry** — Address stored by `configure_review_security`; review submission validates a `get_completion_proof` result against the recomputed interaction commitment.
- **Review token (Soroban token)** — Configured stake token; `submit_review` transfers `base_stake * rating` from the learner via `token::Client::transfer`.
- **`shared` crate** (`../shared`) — Supplies `EscrowRecord`, `EscrowStatus`, `ReputationProof`, and all welfare/fraud/market analysis types and pure functions (e.g. `analyze_review_pattern`, `detect_predatory_behavior`, `calculate_burnout_risk`).

## Testing

```bash
cargo test -p mentorminds-reputation
```

Tests include a mock escrow contract (`MockEscrow`) that serves `get_escrow_by_session` to exercise the Released-session gate.
