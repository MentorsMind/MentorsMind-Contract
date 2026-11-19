//! Peer authentication token rotation (#1490).
//!
//! A peer authenticates to the network with a token. Tokens used to live
//! forever, so a leaked token stayed valid until someone noticed. Every token
//! now records when it was issued and stops being accepted once
//! [`TOKEN_ROTATION_INTERVAL_SECS`] has passed, and [`rotate_peer_token`]
//! replaces a peer's token with a fresh one, which invalidates the old one at
//! the same moment.
//!
//! # Lifecycle
//!
//! 1. [`issue_peer_token`] gives a peer its first token (`generation` 0).
//! 2. [`validate_peer_token`] accepts the presented token only while it is the
//!    peer's current token and is younger than the rotation interval.
//! 3. [`rotate_peer_token`] issues generation `n + 1` stamped with the current
//!    ledger time. The previous token no longer matches what is stored, so it
//!    is rejected immediately, not at its own expiry. An expired token can be
//!    rotated, which is how a peer renews.
//!
//! # What a token is
//!
//! On-chain data is public, so a token here is a freshness-checked identifier,
//! not a secret: it is derived from the peer id, issue time, generation and
//! ledger sequence, so every rotation produces a different value. Anything that
//! must stay secret has to be kept off-chain and bound to the token.
//!
//! # Authorization
//!
//! These functions manage token state only. The contract that calls
//! [`issue_peer_token`] or [`rotate_peer_token`] must first authorize the
//! caller (for example by requiring the peer's own signature or an admin's).
//!
//! All functions touch persistent storage, so they must run inside a contract
//! context.

use soroban_sdk::{contracterror, contracttype, xdr::ToXdr, Bytes, BytesN, Env, Symbol};

/// How long a peer token is accepted after it is issued (24 hours).
pub const TOKEN_ROTATION_INTERVAL_SECS: u64 = 86_400;

const TTL_THRESHOLD: u32 = 518_400;
const TTL_BUMP: u32 = 3_110_400;

#[contracttype]
#[derive(Clone)]
pub enum NetworkAuthKey {
    /// The current token of a peer.
    PeerToken(Symbol),
}

/// A peer's current authentication token.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerAuthToken {
    pub peer_id: Symbol,
    pub token: BytesN<32>,
    /// Ledger timestamp (seconds) at which this token was issued.
    pub issued_at: u64,
    /// Number of rotations so far; 0 for the first token.
    pub generation: u32,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum NetworkAuthError {
    /// `issue_peer_token` was called for a peer that already has a token; use
    /// `rotate_peer_token` instead.
    TokenAlreadyIssued = 1,
    /// The peer has no token.
    TokenNotFound = 2,
    /// The presented token is not the peer's current token (never issued, or
    /// replaced by a rotation).
    TokenInvalid = 3,
    /// The token is the current one but older than the rotation interval.
    TokenExpired = 4,
}

/// Timestamp from which `token` is no longer accepted.
pub fn token_expires_at(token: &PeerAuthToken) -> u64 {
    token.issued_at.saturating_add(TOKEN_ROTATION_INTERVAL_SECS)
}

/// Whether `token` has outlived the rotation interval at time `now`. A token
/// is expired from the exact second the interval has elapsed.
pub fn is_token_expired(token: &PeerAuthToken, now: u64) -> bool {
    now >= token_expires_at(token)
}

fn derive_token(env: &Env, peer_id: &Symbol, issued_at: u64, generation: u32) -> BytesN<32> {
    let mut data = Bytes::new(env);
    data.extend_from_array(&issued_at.to_be_bytes());
    data.extend_from_array(&generation.to_be_bytes());
    data.extend_from_array(&env.ledger().sequence().to_be_bytes());
    data.append(&peer_id.clone().to_xdr(env));
    env.crypto().sha256(&data).to_bytes()
}

fn store(env: &Env, token: &PeerAuthToken) {
    let key = NetworkAuthKey::PeerToken(token.peer_id.clone());
    env.storage().persistent().set(&key, token);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
}

/// The peer's current token, if it has one.
pub fn get_peer_token(env: &Env, peer_id: &Symbol) -> Option<PeerAuthToken> {
    env.storage()
        .persistent()
        .get(&NetworkAuthKey::PeerToken(peer_id.clone()))
}

/// Issue a peer's first token, stamped with the current ledger time.
///
/// # Errors
/// [`NetworkAuthError::TokenAlreadyIssued`] if the peer already has one.
pub fn issue_peer_token(env: &Env, peer_id: &Symbol) -> Result<PeerAuthToken, NetworkAuthError> {
    if get_peer_token(env, peer_id).is_some() {
        return Err(NetworkAuthError::TokenAlreadyIssued);
    }
    let issued_at = env.ledger().timestamp();
    let token = PeerAuthToken {
        peer_id: peer_id.clone(),
        token: derive_token(env, peer_id, issued_at, 0),
        issued_at,
        generation: 0,
    };
    store(env, &token);
    Ok(token)
}

/// Check a presented token.
///
/// # Errors
/// * [`NetworkAuthError::TokenNotFound`]: the peer has no token.
/// * [`NetworkAuthError::TokenInvalid`]: `presented` is not the current token.
/// * [`NetworkAuthError::TokenExpired`]: it is the current token but is older
///   than [`TOKEN_ROTATION_INTERVAL_SECS`].
pub fn validate_peer_token(
    env: &Env,
    peer_id: &Symbol,
    presented: &BytesN<32>,
) -> Result<(), NetworkAuthError> {
    let current = get_peer_token(env, peer_id).ok_or(NetworkAuthError::TokenNotFound)?;
    if &current.token != presented {
        return Err(NetworkAuthError::TokenInvalid);
    }
    if is_token_expired(&current, env.ledger().timestamp()) {
        return Err(NetworkAuthError::TokenExpired);
    }
    Ok(())
}

/// Replace a peer's token with a fresh one and invalidate the old one.
///
/// The new token has `generation + 1` and `issued_at` set to the current
/// ledger time. Because the stored token is overwritten, the previous token is
/// rejected from this moment on. Works on an expired token too.
///
/// # Errors
/// [`NetworkAuthError::TokenNotFound`] if the peer has no token to rotate.
pub fn rotate_peer_token(env: &Env, peer_id: &Symbol) -> Result<PeerAuthToken, NetworkAuthError> {
    let previous = get_peer_token(env, peer_id).ok_or(NetworkAuthError::TokenNotFound)?;
    let issued_at = env.ledger().timestamp();
    let generation = previous.generation.saturating_add(1);
    let token = PeerAuthToken {
        peer_id: peer_id.clone(),
        token: derive_token(env, peer_id, issued_at, generation),
        issued_at,
        generation,
    };
    store(env, &token);
    Ok(token)
}

/// Whether the peer has a token that has outlived the rotation interval.
/// `false` for a peer with no token.
pub fn peer_token_needs_rotation(env: &Env, peer_id: &Symbol) -> bool {
    get_peer_token(env, peer_id)
        .map(|token| is_token_expired(&token, env.ledger().timestamp()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{symbol_short, testutils::Ledger, Address};

    #[soroban_sdk::contract]
    struct DummyNetworkContract;

    #[soroban_sdk::contractimpl]
    impl DummyNetworkContract {}

    fn env_at(ts: u64) -> (Env, Address) {
        let env = Env::default();
        env.ledger().with_mut(|l| l.timestamp = ts);
        let contract_id = env.register_contract(None, DummyNetworkContract);
        (env, contract_id)
    }

    fn advance(env: &Env, secs: u64) {
        env.ledger().with_mut(|l| {
            l.timestamp += secs;
            l.sequence_number += 1;
        });
    }

    #[test]
    fn issued_token_records_the_issue_time() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let peer = symbol_short!("peer1");
            let token = issue_peer_token(&env, &peer).unwrap();

            assert_eq!(token.issued_at, 1_000);
            assert_eq!(token.generation, 0);
            assert_eq!(
                token_expires_at(&token),
                1_000 + TOKEN_ROTATION_INTERVAL_SECS
            );
            assert_eq!(get_peer_token(&env, &peer), Some(token));
        });
    }

    #[test]
    fn a_peer_cannot_be_issued_a_second_first_token() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let peer = symbol_short!("peer1");
            issue_peer_token(&env, &peer).unwrap();
            assert_eq!(
                issue_peer_token(&env, &peer),
                Err(NetworkAuthError::TokenAlreadyIssued)
            );
        });
    }

    #[test]
    fn token_is_valid_until_the_rotation_interval_elapses() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let peer = symbol_short!("peer1");
            let token = issue_peer_token(&env, &peer).unwrap();

            assert_eq!(validate_peer_token(&env, &peer, &token.token), Ok(()));

            advance(&env, TOKEN_ROTATION_INTERVAL_SECS - 1);
            assert_eq!(validate_peer_token(&env, &peer, &token.token), Ok(()));
            assert!(!is_token_expired(&token, env.ledger().timestamp()));
            assert!(!peer_token_needs_rotation(&env, &peer));
        });
    }

    #[test]
    fn token_expires_exactly_at_the_interval() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let peer = symbol_short!("peer1");
            let token = issue_peer_token(&env, &peer).unwrap();

            advance(&env, TOKEN_ROTATION_INTERVAL_SECS);
            assert!(is_token_expired(&token, env.ledger().timestamp()));
            assert_eq!(
                validate_peer_token(&env, &peer, &token.token),
                Err(NetworkAuthError::TokenExpired)
            );
            assert!(peer_token_needs_rotation(&env, &peer));
        });
    }

    #[test]
    fn rotation_issues_a_fresh_token_and_invalidates_the_old_one() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let peer = symbol_short!("peer1");
            let old = issue_peer_token(&env, &peer).unwrap();

            advance(&env, 600);
            let new = rotate_peer_token(&env, &peer).unwrap();

            assert_ne!(new.token, old.token);
            assert_eq!(new.generation, 1);
            assert_eq!(new.issued_at, 1_600);
            // The old token is rejected at once, well before its own expiry.
            assert_eq!(
                validate_peer_token(&env, &peer, &old.token),
                Err(NetworkAuthError::TokenInvalid)
            );
            assert_eq!(validate_peer_token(&env, &peer, &new.token), Ok(()));
            assert_eq!(get_peer_token(&env, &peer), Some(new));
        });
    }

    #[test]
    fn rotating_an_expired_token_renews_the_peer() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let peer = symbol_short!("peer1");
            let old = issue_peer_token(&env, &peer).unwrap();

            advance(&env, TOKEN_ROTATION_INTERVAL_SECS + 5);
            assert_eq!(
                validate_peer_token(&env, &peer, &old.token),
                Err(NetworkAuthError::TokenExpired)
            );

            let new = rotate_peer_token(&env, &peer).unwrap();
            assert_eq!(validate_peer_token(&env, &peer, &new.token), Ok(()));
            assert!(!peer_token_needs_rotation(&env, &peer));
            // The restarted clock runs from the rotation, not the first issue.
            assert_eq!(new.issued_at, 1_000 + TOKEN_ROTATION_INTERVAL_SECS + 5);
        });
    }

    #[test]
    fn every_rotation_yields_a_distinct_token_even_within_one_ledger() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let peer = symbol_short!("peer1");
            let first = issue_peer_token(&env, &peer).unwrap();
            let second = rotate_peer_token(&env, &peer).unwrap();
            let third = rotate_peer_token(&env, &peer).unwrap();

            assert_ne!(first.token, second.token);
            assert_ne!(second.token, third.token);
            assert_eq!(third.generation, 2);
            assert_eq!(
                validate_peer_token(&env, &peer, &second.token),
                Err(NetworkAuthError::TokenInvalid)
            );
        });
    }

    #[test]
    fn rotating_a_peer_with_no_token_fails() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let peer = symbol_short!("ghost");
            assert_eq!(
                rotate_peer_token(&env, &peer),
                Err(NetworkAuthError::TokenNotFound)
            );
            assert!(!peer_token_needs_rotation(&env, &peer));
        });
    }

    #[test]
    fn validating_an_unknown_peer_or_a_foreign_token_fails() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let alice = symbol_short!("alice");
            let bob = symbol_short!("bob");
            let alice_token = issue_peer_token(&env, &alice).unwrap();
            let bob_token = issue_peer_token(&env, &bob).unwrap();

            assert_eq!(
                validate_peer_token(&env, &symbol_short!("nobody"), &alice_token.token),
                Err(NetworkAuthError::TokenNotFound)
            );
            assert_ne!(alice_token.token, bob_token.token);
            assert_eq!(
                validate_peer_token(&env, &alice, &bob_token.token),
                Err(NetworkAuthError::TokenInvalid)
            );
        });
    }

    #[test]
    fn rotating_one_peer_leaves_other_peers_untouched() {
        let (env, id) = env_at(1_000);
        env.as_contract(&id, || {
            let alice = symbol_short!("alice");
            let bob = symbol_short!("bob");
            issue_peer_token(&env, &alice).unwrap();
            let bob_token = issue_peer_token(&env, &bob).unwrap();

            rotate_peer_token(&env, &alice).unwrap();

            assert_eq!(validate_peer_token(&env, &bob, &bob_token.token), Ok(()));
            assert_eq!(get_peer_token(&env, &bob).unwrap().generation, 0);
        });
    }

    #[test]
    fn expiry_arithmetic_saturates_instead_of_overflowing() {
        let token = PeerAuthToken {
            peer_id: symbol_short!("p"),
            token: BytesN::from_array(&Env::default(), &[0u8; 32]),
            issued_at: u64::MAX - 1,
            generation: 0,
        };
        assert_eq!(token_expires_at(&token), u64::MAX);
        assert!(is_token_expired(&token, u64::MAX));
        assert!(!is_token_expired(&token, 0));
    }
}
