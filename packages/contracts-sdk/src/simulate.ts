export interface SimulationStateChange {
  key: string;
  before: unknown;
  after: unknown;
  type: "created" | "updated" | "deleted" | "unknown";
}

export interface SimulationResult {
  success: boolean;
  fee_estimate: bigint;
  state_changes: SimulationStateChange[];
  error?: string;
}

export interface SimulationOptions {
  endpoint: string;
  timeoutMs?: number;
}

type SimulatePayload = {
  transaction: string;
  sourceAccount?: string;
};

function normalizeError(err: unknown): string {
  if (err instanceof Error) {
    return err.message;
  }
  return "Unknown simulation error";
}

function parseFeeEstimate(raw: any): bigint {
  const feeCandidate =
    raw?.minResourceFee ??
    raw?.result?.minResourceFee ??
    raw?.fee ??
    raw?.result?.fee ??
    raw?.transactionData?.resourceFee ??
    0;

  try {
    return BigInt(feeCandidate);
  } catch {
    return 0n;
  }
}

function asChangeType(raw: any): SimulationStateChange["type"] {
  const kind = String(raw?.type ?? raw?.changeType ?? "").toLowerCase();
  if (kind.includes("create")) return "created";
  if (kind.includes("update") || kind.includes("modify")) return "updated";
  if (kind.includes("delete")) return "deleted";
  return "unknown";
}

export function parseStateChanges(raw: any): SimulationStateChange[] {
  const candidates =
    raw?.stateChanges ??
    raw?.state_changes ??
    raw?.result?.stateChanges ??
    raw?.result?.state_changes ??
    [];

  if (!Array.isArray(candidates)) {
    return [];
  }

  return candidates.map((item: any, idx: number) => ({
    key: String(item?.key ?? item?.ledgerKey ?? `change-${idx}`),
    before: item?.before ?? item?.prev ?? null,
    after: item?.after ?? item?.next ?? null,
    type: asChangeType(item),
  }));
}

async function postWithTimeout(url: string, body: unknown, timeoutMs = 15_000): Promise<Response> {
  const controller = new AbortController();
  const id = setTimeout(() => controller.abort(), timeoutMs);

  try {
    return await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
  } finally {
    clearTimeout(id);
  }
}

export async function simulateTransaction(
  operation: string,
  account: string,
  options: SimulationOptions,
): Promise<SimulationResult> {
  const payload: SimulatePayload = {
    transaction: operation,
    sourceAccount: account,
  };

  try {
    const rpcBody = {
      jsonrpc: "2.0",
      id: "simulate",
      method: "simulateTransaction",
      params: {
        transaction: operation,
      },
    };

    const rpcRes = await postWithTimeout(options.endpoint, rpcBody, options.timeoutMs);
    if (rpcRes.ok) {
      const rpcJson = await rpcRes.json();
      const result = rpcJson?.result ?? rpcJson;
      const error = rpcJson?.error?.message;

      if (error) {
        return {
          success: false,
          fee_estimate: parseFeeEstimate(result),
          state_changes: parseStateChanges(result),
          error,
        };
      }

      return {
        success: true,
        fee_estimate: parseFeeEstimate(result),
        state_changes: parseStateChanges(result),
      };
    }

    // Fallback for Horizon-style REST endpoint naming used by some stacks.
    const restRes = await postWithTimeout(`${options.endpoint.replace(/\/$/, "")}/simulate_transaction`, payload, options.timeoutMs);
    const restJson = await restRes.json();

    if (!restRes.ok || restJson?.error) {
      return {
        success: false,
        fee_estimate: parseFeeEstimate(restJson),
        state_changes: parseStateChanges(restJson),
        error: String(restJson?.error ?? `Simulation failed (${restRes.status})`),
      };
    }

    return {
      success: true,
      fee_estimate: parseFeeEstimate(restJson),
      state_changes: parseStateChanges(restJson),
    };
  } catch (err) {
    return {
      success: false,
      fee_estimate: 0n,
      state_changes: [],
      error: normalizeError(err),
    };
  }
}

export async function estimateFee(
  operation: string,
  options: SimulationOptions,
  account = "",
): Promise<bigint> {
  const result = await simulateTransaction(operation, account, options);
  return result.fee_estimate;
}
