#![allow(dead_code)]
use soroban_sdk::{Bytes, BytesN, Env};

pub const BADGE_FIRST_SESSION: &str = "FirstSession";
pub const BADGE_TEN_SESSIONS: &str = "TenSessions";
pub const BADGE_HUNDRED_SESSIONS: &str = "HundredSessions";
pub const BADGE_TOP_RATED: &str = "TopRated";
pub const BADGE_VERIFIED_EXPERT: &str = "VerifiedExpert";
pub const BADGE_EARLY_ADOPTER: &str = "EarlyAdopter";
pub const BADGE_COMMUNITY_LEADER: &str = "CommunityLeader";

pub fn badge_type_hash(env: &Env, name: &str) -> BytesN<32> {
    let bytes = Bytes::from_slice(env, name.as_bytes());
    env.crypto().sha256(&bytes).into()
}