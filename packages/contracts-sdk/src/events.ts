import { ContractEvent, HorizonEvent } from "./event-types";

export const EVENT_TOPICS = {
  ESCROW: ["Escrow", "escrow"],
  MNT_TOKEN: ["MNTToken", "mnttoken"],
  REFERRAL: ["Referral", "referral"],
  VERIFICATION: ["Verification", "verification"],
  ORACLE: ["oracle", "Oracle"],
  STAKING: ["reward", "staking"],
  TIMELOCK: ["timelock", "Timelock"],
  TREASURY: ["buyback", "treasury"],
  GOVERNANCE: ["governance", "Governance"],
  ALLOWANCE: ["allowance", "Allowance"],
  BRIDGE: ["bridge", "Bridge"],
} as const;

function decodeScValXdr(base64Xdr?: string): unknown {
  if (!base64Xdr) return null;

  // Runtime-friendly fallback:
  // - If an app wires a global Stellar SDK decoder, we use it.
  // - Otherwise return the raw XDR for callers that decode elsewhere.
  const maybeDecoder = (globalThis as any).__stellarDecodeScVal;
  if (typeof maybeDecoder === "function") {
    try {
      return maybeDecoder(base64Xdr);
    } catch {
      return null;
    }
  }

  return { rawXdr: base64Xdr };
}

function decodeTopics(raw: HorizonEvent): unknown[] {
  if (Array.isArray(raw.topic)) {
    return raw.topic;
  }

  if (Array.isArray(raw.topic_xdr)) {
    return raw.topic_xdr.map((t) => decodeScValXdr(t));
  }

  return [];
}

function decodeValue(raw: HorizonEvent): any {
  if (raw.value !== undefined) {
    return raw.value;
  }
  return decodeScValXdr(raw.value_xdr);
}

function normalizeTopic(topic: unknown): string {
  return String(topic ?? "").replace(/^Symbol\(|\)$/g, "").trim();
}

function toStr(v: unknown): string {
  if (v === null || v === undefined) return "";
  return typeof v === "bigint" ? v.toString() : String(v);
}

export function decodeEvent(raw: HorizonEvent): ContractEvent | null {
  try {
    const topics = decodeTopics(raw);
    if (topics.length < 2) {
      return null;
    }

    const contractTopic = normalizeTopic(topics[0]);
    const eventTopic = normalizeTopic(topics[1]);
    const entity = topics[2];
    const data = decodeValue(raw);

    const base = {
      contractId: raw.contract_id,
      ledger: raw.ledger,
      txHash: raw.tx_hash,
    };

    if (EVENT_TOPICS.GOVERNANCE.includes(contractTopic as any)) {
      if (eventTopic === "proposal_created") {
        const tuple = Array.isArray(data) ? data : [];
        return {
          ...base,
          kind: "ProposalCreated",
          proposalId: Number(entity ?? 0),
          proposer: toStr(tuple[0]),
        };
      }

      if (eventTopic === "vote_cast") {
        const tuple = Array.isArray(data) ? data : [];
        return {
          ...base,
          kind: "VoteCast",
          proposalId: Number(entity ?? 0),
          voter: toStr(tuple[0]),
          support: Boolean(tuple[1]),
          weight: toStr(tuple[2]),
        };
      }

      if (eventTopic === "proposal_executed") {
        return {
          ...base,
          kind: "ProposalExecuted",
          proposalId: Number(entity ?? 0),
        };
      }
    }

    if (EVENT_TOPICS.ALLOWANCE.includes(contractTopic as any)) {
      const tuple = Array.isArray(data) ? data : [];
      if (eventTopic === "authorized") {
        return {
          ...base,
          kind: "AllowanceAuthorized",
          owner: toStr(entity),
          spender: toStr(tuple[0]),
          token: toStr(tuple[1]),
          amount: toStr(tuple[2]),
        };
      }

      if (eventTopic === "payment_pulled") {
        return {
          ...base,
          kind: "AllowancePaymentPulled",
          owner: toStr(entity),
          spender: toStr(tuple[0]),
          token: toStr(tuple[1]),
          amount: toStr(tuple[2]),
        };
      }

      if (eventTopic === "revoked") {
        return {
          ...base,
          kind: "AllowanceRevoked",
          owner: toStr(entity),
          spender: toStr(tuple[0]),
          token: toStr(tuple[1]),
        };
      }
    }

    if (EVENT_TOPICS.MNT_TOKEN.includes(contractTopic as any)) {
      if (eventTopic === "Mint") {
        return {
          ...base,
          kind: "Mint",
          to: toStr(entity),
          amount: toStr((data as any)?.amount),
        };
      }
      if (eventTopic === "Burn") {
        return {
          ...base,
          kind: "Burn",
          from: toStr(entity),
          amount: toStr((data as any)?.amount),
        };
      }
      if (eventTopic === "Transfer") {
        return {
          ...base,
          kind: "Transfer",
          from: toStr(entity),
          to: toStr((data as any)?.to),
          amount: toStr((data as any)?.amount),
        };
      }
      if (eventTopic === "Approve") {
        return {
          ...base,
          kind: "Approve",
          from: toStr(entity),
          spender: toStr((data as any)?.spender),
          amount: toStr((data as any)?.amount),
        };
      }
    }

    if (EVENT_TOPICS.ESCROW.includes(contractTopic as any)) {
      if (eventTopic === "Created") {
        return {
          ...base,
          kind: "EscrowCreated",
          escrowId: toStr(entity),
          mentor: toStr((data as any)?.mentor),
          learner: toStr((data as any)?.learner),
          amount: toStr((data as any)?.amount),
          sessionId: toStr((data as any)?.session_id),
          tokenAddress: toStr((data as any)?.token_address),
        };
      }
      if (eventTopic === "Released") {
        return {
          ...base,
          kind: "EscrowReleased",
          escrowId: toStr(entity),
          mentor: toStr((data as any)?.mentor),
          amount: toStr((data as any)?.amount),
          netAmount: toStr((data as any)?.net_amount),
          platformFee: toStr((data as any)?.platform_fee),
          tokenAddress: toStr((data as any)?.token_address),
        };
      }
      if (eventTopic === "ReviewSubmitted") {
        return {
          ...base,
          kind: "ReviewSubmitted",
          escrowId: toStr(entity),
          caller: toStr((data as any)?.caller),
          reason: toStr((data as any)?.reason),
          mentor: toStr((data as any)?.mentor),
        };
      }
    }

    if (EVENT_TOPICS.REFERRAL.includes(contractTopic as any)) {
      if (eventTopic === "Registered") {
        return {
          ...base,
          kind: "ReferralRegistered",
          referrer: toStr(entity),
          referee: toStr((data as any)?.referee),
          isMentor: Boolean((data as any)?.is_mentor),
        };
      }
      if (eventTopic === "RewardClaimed") {
        return {
          ...base,
          kind: "RewardClaimed",
          referrer: toStr(entity),
          amount: toStr((data as any)?.amount),
        };
      }
    }

    if (EVENT_TOPICS.VERIFICATION.includes(contractTopic as any)) {
      if (eventTopic === "Verified") {
        return {
          ...base,
          kind: "MentorVerified",
          mentor: toStr(entity),
          credentialHash: toStr((data as any)?.credential_hash),
          expiry: Number((data as any)?.expiry ?? 0),
        };
      }
      if (eventTopic === "Revoked") {
        return {
          ...base,
          kind: "VerificationRevoked",
          mentor: toStr(entity),
        };
      }
    }

    if (EVENT_TOPICS.ORACLE.includes(contractTopic as any) && eventTopic === "price_upd") {
      const tuple = Array.isArray(data) ? data : [];
      return {
        ...base,
        kind: "OraclePriceUpdated",
        asset: toStr(entity),
        price: toStr(tuple[0]),
        timestamp: Number(tuple[1] ?? 0),
      };
    }

    if (contractTopic === "reward" || contractTopic === "claimed") {
      if (contractTopic === "reward") {
        return {
          ...base,
          kind: "RewardsDistributed",
          token: toStr(topics[1]),
          totalAmount: toStr((data as any)?.total_amount),
          totalStaked: toStr((data as any)?.total_staked),
        };
      }

      return {
        ...base,
        kind: "RewardsClaimed",
        token: toStr(topics[1]),
        staker: toStr((data as any)?.staker),
        amount: toStr((data as any)?.amount),
      };
    }

    if (EVENT_TOPICS.TIMELOCK.includes(contractTopic as any)) {
      if (eventTopic === "scheduled") {
        return {
          ...base,
          kind: "TimelockScheduled",
          operationId: toStr(entity),
        };
      }
      if (eventTopic === "executed") {
        return {
          ...base,
          kind: "TimelockExecuted",
          operationId: toStr(entity),
        };
      }
      if (eventTopic === "cancelled") {
        return {
          ...base,
          kind: "TimelockCancelled",
          operationId: toStr(entity),
        };
      }
    }

    if (contractTopic === "buyback") {
      return {
        ...base,
        kind: "BuybackExecuted",
        usdcSpent: toStr((data as any)?.usdc_spent),
        mntBurned: toStr((data as any)?.mnt_burned),
        price: toStr((data as any)?.price),
      };
    }

    if (EVENT_TOPICS.BRIDGE.includes(contractTopic as any) && eventTopic === "asset_bridged") {
      return {
        ...base,
        kind: "BridgeAssetBridged",
        vaaHash: toStr((data as any)?.vaa_hash),
        recipient: toStr((data as any)?.recipient),
        amount: toStr((data as any)?.amount),
        sourceChain: Number((data as any)?.source_chain ?? 0),
        wrappedToken: toStr((data as any)?.wrapped_token),
      };
    }

    console.warn("[contracts-sdk/events] Unknown event", {
      contractTopic,
      eventTopic,
      raw,
    });
    return null;
  } catch (err) {
    console.warn("[contracts-sdk/events] Failed to decode event", err);
    return null;
  }
}
