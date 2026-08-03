import assert from "node:assert/strict";
import test from "node:test";

import { correlateControlResultFrame } from "./controlResultCorrelation.ts";

test("deferred model result preserves the originally targeted turn", () => {
  assert.deepEqual(
    correlateControlResultFrame(
      {
        type: "switch_model",
        status: "switched",
        modelId: "model-b",
        turnId: "original-turn-a",
      },
      "replacement-turn-b",
    ),
    {
      type: "switch_model",
      status: "switched",
      modelId: "model-b",
      turnId: "original-turn-a",
    },
  );
});

test("envelope turn id is used when a legacy result omits payload correlation", () => {
  assert.deepEqual(
    correlateControlResultFrame(
      { type: "switch_model", status: "unsupported_model", modelId: "x" },
      "turn-a",
    ),
    {
      type: "switch_model",
      status: "unsupported_model",
      modelId: "x",
      turnId: "turn-a",
    },
  );
});

test("contradictory immediate acknowledgement is rejected", () => {
  assert.equal(
    correlateControlResultFrame(
      { type: "switch_model", status: "sent", turnId: "turn-a" },
      "turn-b",
    ),
    null,
  );
});

test("oversized payload turn id cannot displace a valid envelope id", () => {
  const frame = correlateControlResultFrame(
    { type: "switch_model", status: "switch_failed", turnId: "x".repeat(129) },
    "turn-a",
  );
  assert.equal(frame?.turnId, "turn-a");
});
