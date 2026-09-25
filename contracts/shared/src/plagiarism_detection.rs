/// Plagiarism Detection Module
///
/// Implements content fingerprinting, similarity analysis, and comprehensive
/// plagiarism detection to identify and prevent content theft while protecting
/// legitimate content creators.

use soroban_sdk::{symbol_short as symbol, Address, Env, Symbol, Vec};

/// Content fingerprint for fast similarity comparison
#[derive(Clone, Debug, PartialEq)]
pub struct ContentFingerprint {
    pub content_hash: Symbol,
    pub fingerprint_hash: Symbol,
    pub created_at: u64,
    pub owner: Address,
    pub content_size_bytes: u32,
    pub confidence_score: u32, // 0-10000 basis points
}

/// Plagiarism detection result
#[derive(Clone, Debug, PartialEq)]
pub struct PlagiarismDetectionResult {
    pub original_content: Symbol,
    pub suspicious_content: Symbol,
    pub similarity_score: u32, // 0-10000 basis points
    pub is_plagiarism: bool,
    pub matching_segments: u32,
    pub total_segments: u32,
    pub detected_at: u64,
}

/// Content segment for detailed comparison
#[derive(Clone, Debug, PartialEq)]
pub struct ContentSegment {
    pub segment_id: u32,
    pub segment_hash: Symbol,
    pub position: u32,
    pub length: u32,
    pub uniqueness_score: u32,
}

/// Plagiarism report record
#[derive(Clone, Debug, PartialEq)]
pub struct PlagiarismReport {
    pub report_id: Symbol,
    pub original_content: Symbol,
    pub plagiarized_content: Symbol,
    pub similarity_score: u32,
    pub reported_by: Address,
    pub reported_at: u64,
    pub status: Symbol, // "investigating", "confirmed", "resolved", "false_positive"
    pub evidence_items: u32,
}

/// Create fingerprint for fast content comparison
pub fn create_fingerprint(
    env: &Env,
    content_hash: Symbol,
    owner: Address,
    content_size_bytes: u32,
) -> ContentFingerprint {
    let current_time = env.ledger().timestamp();

    // Generate fingerprint by hashing the content hash multiple times
    // with different salts to create a unique signature
    let mut fingerprint_data: Vec<u8> = env.to_bytes(&content_hash).unwrap_or_default();
    fingerprint_data.append(&mut env.to_bytes(&content_size_bytes).unwrap_or_default());
    fingerprint_data.append(&mut env.to_bytes(&current_time).unwrap_or_default());

    let fingerprint_hash = Symbol::short(
        &env.compute_hash_sha256(&fingerprint_data)
            .to_short_string()
            .slice(0..7),
    );

    ContentFingerprint {
        content_hash,
        fingerprint_hash,
        created_at: current_time,
        owner,
        content_size_bytes,
        confidence_score: 10_000, // Maximum confidence for newly created fingerprints
    }
}

/// Compare two fingerprints for similarity
pub fn compare_fingerprints(
    fp1: &ContentFingerprint,
    fp2: &ContentFingerprint,
) -> u32 {
    // Simple byte-level comparison of fingerprints
    if fp1.fingerprint_hash == fp2.fingerprint_hash {
        return 10_000; // 100% similarity - identical fingerprints
    }

    // For different fingerprints, calculate partial similarity
    // based on confidence and size similarity
    let size_diff = if fp1.content_size_bytes > fp2.content_size_bytes {
        fp1.content_size_bytes - fp2.content_size_bytes
    } else {
        fp2.content_size_bytes - fp1.content_size_bytes
    };

    let max_size = if fp1.content_size_bytes > fp2.content_size_bytes {
        fp1.content_size_bytes
    } else {
        fp2.content_size_bytes
    };

    if max_size == 0 {
        return 0;
    }

    // Size-based similarity
    let size_similarity = ((max_size.saturating_sub(size_diff)) as u32)
        .saturating_mul(10_000)
        .saturating_div(max_size as u32);

    // Confidence-weighted similarity
    (size_similarity as u128)
        .saturating_mul(fp1.confidence_score as u128)
        .saturating_div(10_000) as u32
}

/// Analyze content segments for plagiarism detection
pub fn analyze_content_segments(
    segments: &Vec<ContentSegment>,
    reference_segments: &Vec<ContentSegment>,
) -> (u32, u32) {
    let mut matching_segments = 0;
    let total_segments = segments.len();

    for segment in segments.iter() {
        for ref_segment in reference_segments.iter() {
            if segment.segment_hash == ref_segment.segment_hash {
                matching_segments += 1;
                break; // Count each segment only once
            }
        }
    }

    (matching_segments as u32, total_segments as u32)
}

/// Detect plagiarism by comparing suspicious content against originals
pub fn detect_plagiarism(
    env: &Env,
    original_content: Symbol,
    suspicious_content: Symbol,
    original_fingerprint: &ContentFingerprint,
    suspicious_fingerprint: &ContentFingerprint,
    original_segments: &Vec<ContentSegment>,
    suspicious_segments: &Vec<ContentSegment>,
) -> PlagiarismDetectionResult {
    // Compare fingerprints
    let fingerprint_similarity = compare_fingerprints(original_fingerprint, suspicious_fingerprint);

    // Analyze segment matches
    let (matching_segments, total_segments) =
        analyze_content_segments(suspicious_segments, original_segments);

    // Calculate overall similarity score
    let segment_match_ratio = if total_segments > 0 {
        (matching_segments as u128)
            .saturating_mul(10_000)
            .saturating_div(total_segments as u128) as u32
    } else {
        0
    };

    // Weighted combination of fingerprint and segment analysis
    let combined_similarity = (fingerprint_similarity as u128)
        .saturating_mul(6) // 60% weight on fingerprint
        .saturating_add(segment_match_ratio as u128 * 4) // 40% weight on segments
        .saturating_div(10) as u32;

    let is_plagiarism = combined_similarity >= PLAGIARISM_CONFIDENCE_THRESHOLD_BPS;

    PlagiarismDetectionResult {
        original_content,
        suspicious_content,
        similarity_score: combined_similarity,
        is_plagiarism,
        matching_segments,
        total_segments,
        detected_at: env.ledger().timestamp(),
    }
}

/// Create plagiarism report for confirmed plagiarism
pub fn create_plagiarism_report(
    env: &Env,
    original_content: Symbol,
    plagiarized_content: Symbol,
    similarity_score: u32,
    reported_by: Address,
    evidence_count: u32,
) -> PlagiarismReport {
    // Generate report ID based on content hashes
    let mut report_data: Vec<u8> = env.to_bytes(&original_content).unwrap_or_default();
    report_data.append(&mut env.to_bytes(&plagiarized_content).unwrap_or_default());
    report_data.append(&mut env.to_bytes(&env.ledger().timestamp()).unwrap_or_default());

    let report_id = Symbol::short(
        &env.compute_hash_sha256(&report_data)
            .to_short_string()
            .slice(0..7),
    );

    PlagiarismReport {
        report_id,
        original_content,
        plagiarized_content,
        similarity_score,
        reported_by,
        reported_at: env.ledger().timestamp(),
        status: symbol("investigating"),
        evidence_items: evidence_count,
    }
}

/// Segment content for detailed plagiarism analysis
pub fn segment_content(
    env: &Env,
    content_hash: Symbol,
    segment_size: u32,
    total_size: u32,
) -> Vec<ContentSegment> {
    let mut segments: Vec<ContentSegment> = Vec::new();
    let num_segments = (total_size + segment_size - 1) / segment_size;

    for i in 0..num_segments {
        let position = i * segment_size;
        let length = if i == num_segments - 1 {
            total_size - position // Last segment may be smaller
        } else {
            segment_size
        };

        // Create segment hash based on position and size
        let mut segment_data: Vec<u8> = env.to_bytes(&content_hash).unwrap_or_default();
        segment_data.append(&mut env.to_bytes(&position).unwrap_or_default());
        segment_data.append(&mut env.to_bytes(&length).unwrap_or_default());

        let segment_hash = Symbol::short(
            &env.compute_hash_sha256(&segment_data)
                .to_short_string()
                .slice(0..7),
        );

        segments.push(ContentSegment {
            segment_id: i,
            segment_hash,
            position,
            length,
            uniqueness_score: 5_000, // Default moderate uniqueness
        });
    }

    segments
}

/// Calculate overall plagiarism risk for content
pub fn assess_plagiarism_risk(
    detection_results: &Vec<PlagiarismDetectionResult>,
) -> u32 {
    if detection_results.is_empty() {
        return 0;
    }

    let mut total_risk: u128 = 0;
    let mut high_risk_count = 0;

    for result in detection_results.iter() {
        if result.is_plagiarism {
            high_risk_count += 1;
            total_risk = total_risk.saturating_add(result.similarity_score as u128);
        }
    }

    if high_risk_count == 0 {
        return 0;
    }

    (total_risk.saturating_div(detection_results.len() as u128)) as u32
}

/// Constants for plagiarism detection
pub const PLAGIARISM_CONFIDENCE_THRESHOLD_BPS: u32 = 7_500; // 75% similarity = plagiarism
pub const MIN_SEGMENT_SIZE: u32 = 256; // bytes
pub const MAX_SEGMENT_SIZE: u32 = 4_096; // bytes
pub const DEFAULT_SEGMENT_SIZE: u32 = 1024; // bytes
pub const PLAGIARISM_REPORT_RETENTION_SECS: u64 = 31_536_000; // 1 year
pub const FINGERPRINT_VALIDITY_SECS: u64 = 63_072_000; // 2 years
pub const HIGH_PLAGIARISM_RISK_THRESHOLD_BPS: u32 = 8_000; // 80%
