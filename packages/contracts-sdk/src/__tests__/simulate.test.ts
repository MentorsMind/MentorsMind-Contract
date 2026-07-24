import test from "node:test";
import assert from "node:assert/strict";

import { estimateFee, parseStateChanges, simulateTransaction } from "../simulate";

test("successful simulation parsing", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (async () =>
    ({
      ok: true,
      json: async () => ({
        result: {
          minResourceFee: "3210",
          stateChanges: [
            {
              key: "k1",
              before: null,
              after: { value: 10 },
              type: "created",
            },
          ],
        },
      }),
    } as Response)) as typeof fetch;

  const out = await simulateTransaction("AAAA", "GABC", { endpoint: "https://rpc.local" });
  assert.equal(out.success, true);
  assert.equal(out.fee_estimate, 3210n);
  assert.equal(out.state_changes.length, 1);

  globalThis.fetch = originalFetch;
});

test("error simulation parsing", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (async () =>
    ({
      ok: true,
      json: async () => ({
        error: { message: "host function error" },
        result: { minResourceFee: "100" },
      }),
    } as Response)) as typeof fetch;

  const out = await simulateTransaction("AAAA", "GABC", { endpoint: "https://rpc.local" });
  assert.equal(out.success, false);
  assert.match(out.error ?? "", /host function error/);

  globalThis.fetch = originalFetch;
});

test("fee estimate helper", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (async () =>
    ({
      ok: true,
      json: async () => ({ result: { minResourceFee: "999" } }),
    } as Response)) as typeof fetch;

  const fee = await estimateFee("BBBB", { endpoint: "https://rpc.local" }, "GXYZ");
  assert.equal(fee, 999n);

  globalThis.fetch = originalFetch;
});

test("state change fallback parsing", () => {
  const changes = parseStateChanges({
    result: {
      state_changes: [{ ledgerKey: "k2", prev: 1, next: 2, changeType: "updated" }],
    },
  });

  assert.equal(changes[0]?.key, "k2");
  assert.equal(changes[0]?.type, "updated");
});
