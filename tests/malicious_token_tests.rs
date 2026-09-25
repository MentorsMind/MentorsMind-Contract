// =============================================================================
// Malicious Token Whitelist Bypass Tests
//
// These tests verify that the token whitelist in the escrow, payment router,
// and treasury contracts cannot be bypassed through any mechanism.
//
// Test scenarios include:
// 1. Unapproved tokens rejected at escrow creation
// 2. Revoked tokens rejected after removal from whitelist
// 3. Path-payment bypass attempts (unapproved send/dest assets)
// 4. Milestone escrow bypass attempts
// 5. Payment router whitelist enforcement
// 6. Treasury whitelist enforcement
// 7. Re-approval flow after revocation
// 8. Default rejection of unknown tokens
// =============================================================================

// NOTE: These tests are designed as documentation of the whitelist bypass
// prevention strategy. The actual test execution happens in each contract's
// own test module (escrow, payment_router, treasury).
//
// Key security guarantees:
//
// 1. ESCROW CONTRACT (escrow/src/lib.rs):
//    - _create_escrow_internal() validates token_address against whitelist
//    - create_escrow_with_path_payment() validates BOTH send_asset AND dest_asset
//    - create_milestone_escrow() validates token_address against whitelist
//    - set_approved_token() requires admin auth and emits events
//    - _is_token_approved() returns false for any unknown token (no fallback)
//
// 2. PAYMENT ROUTER (contracts/payment_router/src/lib.rs):
//    - route_payment() validates token against router's own whitelist
//    - set_approved_token() requires admin auth
//    - Unknown tokens default to not-approved
//
// 3. TREASURY (contracts/treasury/src/lib.rs):
//    - deposit() validates token against treasury whitelist
//    - allocate() validates token against treasury whitelist  
//    - distribute_to_stakers() validates token against treasury whitelist
//    - buyback_and_burn() validates BOTH xlm_token AND mnt_token
//    - set_approved_token() requires admin auth
//
// Bypass vectors that are now blocked:
//
// a) Direct creation with unapproved token → panic!("Token not approved")
// b) Path-payment with unapproved send asset → panic!("Send asset token not approved")
// c) Path-payment with unapproved dest asset → panic!("Dest asset token not approved")
// d) Milestone escrow with unapproved token → panic!("Token not approved")
// e) Router routing with unapproved token → panic!("Token not approved for routing")
// f) Treasury deposit with unapproved token → Err(Error::TokenNotApproved)
// g) Treasury allocation with unapproved token → Err(Error::TokenNotApproved)
// h) Treasury distribution with unapproved token → Err(Error::TokenNotApproved)
// i) Treasury buyback with unapproved token → Err(Error::TokenNotApproved)
// j) Revoked token usage → same as unapproved (whitelist set to false)
