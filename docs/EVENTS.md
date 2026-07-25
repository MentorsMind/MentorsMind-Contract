# MentorsMind Smart Contract Events Schema

All events emitted by the MentorsMind smart contracts adhere to a standardized topic and data structure to allow reliable sub-query/indexing.

## Standard Schema

*   **Topics:** `(ContractName: Symbol, EventType: Symbol, EntityId: Val)`
*   **Data:** Typed struct wrapping the relevant data for that event type to reconstruct the state without requiring a DB lookup.

---

## 1. MNT-Token (`MNTToken`)

### `Mint`
*   **EntityId:** `to: Address` (Account receiving tokens)
*   **Data:**
    ```rust
    struct MintEventData {
        pub amount: i128,
    }
    ```

### `Burn`
*   **EntityId:** `from: Address` (Account burning tokens)
*   **Data:**
    ```rust
    struct BurnEventData {
        pub amount: i128,
    }
    ```

### `Approve`
*   **EntityId:** `from: Address` (Account granting allowance)
*   **Data:**
    ```rust
    struct ApproveEventData {
        pub spender: Address,
        pub amount: i128,
    }
    ```

### `Transfer`
*   **EntityId:** `from: Address` (Account sending tokens)
*   **Data:**
    ```rust
    struct TransferEventData {
        pub to: Address,
        pub amount: i128,
    }
    ```

---

## 2. Referral (`Referral`)

### `Registered`
*   **EntityId:** `referrer: Address` (Account who referred a user)
*   **Data:**
    ```rust
    struct ReferralRegisteredEventData {
        pub referee: Address,
        pub is_mentor: bool,
    }
    ```

### `RewardClaimed`
*   **EntityId:** `referrer: Address` (Account claiming rewards)
*   **Data:**
    ```rust
    struct RewardClaimedEventData {
        pub amount: i128,
    }
    ```

---

## 3. Verification (`Verification`)

### `Verified`
*   **EntityId:** `mentor: Address` (Mentor being verified)
*   **Data:**
    ```rust
    struct MentorVerifiedEventData {
        pub credential_hash: BytesN<32>,
        pub verified_at: u64,
        pub expiry: u64,
    }
    ```

### `Revoked`
*   **EntityId:** `mentor: Address` (Mentor whose verification was revoked)
*   **Data:**
    ```rust
    struct VerificationRevokedEventData {}
    ```

---

## 4. Escrow (`Escrow`)

### `Created`
*   **EntityId:** `escrow_id: u64`
*   **Data:**
    ```rust
    struct EscrowCreatedEventData {
        pub mentor: Address,
        pub learner: Address,
        pub amount: i128,
        pub session_id: Symbol,
        pub token_address: Address,
        pub session_end_time: u64,
    }
    ```

### `Released`
*   **EntityId:** `escrow_id: u64`
*   **Data:**
    ```rust
    struct EscrowReleasedEventData {
        pub mentor: Address,
        pub amount: i128,
        pub net_amount: i128,
        pub platform_fee: i128,
        pub token_address: Address,
    }
    ```

### `AutoReleased`
*   **EntityId:** `escrow_id: u64`
*   **Data:**
    ```rust
    struct EscrowAutoReleasedEventData {
        pub time: u64,
    }
    ```

### `DisputeOpened`
*   **EntityId:** `escrow_id: u64`
*   **Data:**
    ```rust
    struct DisputeOpenedEventData {
        pub caller: Address,
        pub reason: Symbol,
        pub token_address: Address,
    }
    ```

### `DisputeResolved`
*   **EntityId:** `escrow_id: u64`
*   **Data:**
    ```rust
    struct DisputeResolvedEventData {
        pub mentor_pct: u32,
        pub mentor_amount: i128,
        pub learner_amount: i128,
        pub token_address: Address,
        pub time: u64,
    }
    ```

### `Refunded`
*   **EntityId:** `escrow_id: u64`
*   **Data:**
    ```rust
    struct EscrowRefundedEventData {
        pub learner: Address,
        pub amount: i128,
        pub token_address: Address,
    }
    ```

### `ReviewSubmitted`
*   **EntityId:** `escrow_id: u64`
*   **Data:**
    ```rust
    struct ReviewSubmittedEventData {
        pub caller: Address,
        pub reason: Symbol,
        pub mentor: Address,
    }
    ```
\n\n## Escrow Notification Events\n- `meta_upd`: Escrow metadata updated\n- `rated`: Rating submitted\n