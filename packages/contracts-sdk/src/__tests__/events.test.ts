import test from "node:test";
import assert from "node:assert/strict";

import { decodeEvent } from "../events";

test("decode governance proposal created", () => {
  const parsed = decodeEvent({
    topic: ["governance", "proposal_created", 1 as any],
    value: ["GPROPOSER", 120, 180],
    contract_id: "C123",
    ledger: 10,
  });

  assert.ok(parsed);
  assert.equal(parsed?.kind, "ProposalCreated");
});

test("decode unknown event returns null", () => {
  const parsed = decodeEvent({
    topic: ["unknown", "noop", "x"],
    value: {},
  });

  assert.equal(parsed, null);
});
