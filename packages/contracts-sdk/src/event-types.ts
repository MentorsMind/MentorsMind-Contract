export interface HorizonEvent {
  id?: string;
  contract_id?: string;
  topic?: string[];
  topic_xdr?: string[];
  value?: unknown;
  value_xdr?: string;
  ledger?: number;
  ledger_close_time?: string;
  tx_hash?: string;
}

export interface EventBase {
  kind: string;
  contractId?: string;
  ledger?: number;
  txHash?: string;
}

export interface EscrowCreatedEvent extends EventBase {
  kind: "EscrowCreated";
  escrowId: string;
  mentor: string;
  learner: string;
  amount: string;
  sessionId: string;
  tokenAddress: string;
}

export interface EscrowReleasedEvent extends EventBase {
  kind: "EscrowReleased";
  escrowId: string;
  mentor: string;
  amount: string;
  netAmount: string;
  platformFee: string;
  tokenAddress: string;
}

export interface ReviewSubmittedEvent extends EventBase {
  kind: "ReviewSubmitted";
  escrowId: string;
  caller: string;
  reason: string;
  mentor: string;
}

export interface MintEvent extends EventBase {
  kind: "Mint";
  to: string;
  amount: string;
}

export interface BurnEvent extends EventBase {
  kind: "Burn";
  from: string;
  amount: string;
}

export interface TransferEvent extends EventBase {
  kind: "Transfer";
  from: string;
  to: string;
  amount: string;
}

export interface ApproveEvent extends EventBase {
  kind: "Approve";
  from: string;
  spender: string;
  amount: string;
}

export interface VerificationEvent extends EventBase {
  kind: "MentorVerified" | "VerificationRevoked";
  mentor: string;
  credentialHash?: string;
  expiry?: number;
}

export interface ReferralEvent extends EventBase {
  kind: "ReferralRegistered" | "RewardClaimed";
  referrer: string;
  referee?: string;
  isMentor?: boolean;
  amount?: string;
}

export interface OraclePriceUpdatedEvent extends EventBase {
  kind: "OraclePriceUpdated";
  asset: string;
  price: string;
  timestamp: number;
}

export interface StakingEvent extends EventBase {
  kind: "RewardsDistributed" | "RewardsClaimed";
  staker?: string;
  token: string;
  amount?: string;
  totalAmount?: string;
  totalStaked?: string;
}

export interface TimelockEvent extends EventBase {
  kind: "TimelockScheduled" | "TimelockExecuted" | "TimelockCancelled";
  operationId: string;
}

export interface TreasuryBuybackEvent extends EventBase {
  kind: "BuybackExecuted";
  usdcSpent: string;
  mntBurned: string;
  price: string;
}

export interface BridgeAssetBridgedEvent extends EventBase {
  kind: "BridgeAssetBridged";
  vaaHash: string;
  recipient: string;
  amount: string;
  sourceChain: number;
  wrappedToken: string;
}

export interface GovernanceEvent extends EventBase {
  kind: "ProposalCreated" | "VoteCast" | "ProposalExecuted";
  proposalId: number;
  proposer?: string;
  voter?: string;
  support?: boolean;
  weight?: string;
}

export interface AllowanceEvent extends EventBase {
  kind: "AllowanceAuthorized" | "AllowancePaymentPulled" | "AllowanceRevoked";
  owner: string;
  spender: string;
  token: string;
  amount?: string;
}

export type ContractEvent =
  | EscrowCreatedEvent
  | EscrowReleasedEvent
  | ReviewSubmittedEvent
  | MintEvent
  | BurnEvent
  | TransferEvent
  | ApproveEvent
  | VerificationEvent
  | ReferralEvent
  | OraclePriceUpdatedEvent
  | StakingEvent
  | TimelockEvent
  | TreasuryBuybackEvent
  | BridgeAssetBridgedEvent
  | GovernanceEvent
  | AllowanceEvent;
