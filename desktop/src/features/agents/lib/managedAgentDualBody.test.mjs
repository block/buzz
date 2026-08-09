/**
 * Entity holon R2 — dual-body refuse when presence says DNA is live elsewhere.
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";

// Mirror refuseDualBodyIfPresentElsewhere logic (pure) for fast unit coverage
// without TS loader — keep in sync with managedAgentControlActions.ts

function normalizePubkey(pk) {
  return String(pk || "").toLowerCase();
}

function isManagedAgentActive(agent) {
  return agent.status === "running" || agent.status === "deployed";
}

function refuseDualBodyIfPresentElsewhere(input) {
  if (input.allowDualBody) return;
  if (input.agent.backend.type !== "local") return;
  if (isManagedAgentActive(input.agent)) return;
  const pk = normalizePubkey(input.agent.pubkey);
  const status =
    input.presenceLookup?.[pk] ?? input.presenceLookup?.[input.agent.pubkey];
  if (status !== "online" && status !== "away") return;
  throw new Error(`Refuse dual body for ${input.agent.name}`);
}

const base = {
  pubkey: "aa".repeat(32),
  name: "Home-Fizz",
  backend: { type: "local" },
  status: "stopped",
};

describe("refuseDualBodyIfPresentElsewhere", () => {
  it("allows start when presence offline/missing", () => {
    assert.doesNotThrow(() =>
      refuseDualBodyIfPresentElsewhere({
        agent: base,
        presenceLookup: {},
      }),
    );
    assert.doesNotThrow(() =>
      refuseDualBodyIfPresentElsewhere({
        agent: base,
        presenceLookup: { [base.pubkey]: "offline" },
      }),
    );
  });

  it("refuses when online elsewhere", () => {
    assert.throws(
      () =>
        refuseDualBodyIfPresentElsewhere({
          agent: base,
          presenceLookup: { [base.pubkey]: "online" },
        }),
      /Refuse dual body/,
    );
  });

  it("refuses when away elsewhere", () => {
    assert.throws(() =>
      refuseDualBodyIfPresentElsewhere({
        agent: base,
        presenceLookup: { [base.pubkey]: "away" },
      }),
    );
  });

  it("skips provider agents", () => {
    assert.doesNotThrow(() =>
      refuseDualBodyIfPresentElsewhere({
        agent: { ...base, backend: { type: "provider", id: "k8s" } },
        presenceLookup: { [base.pubkey]: "online" },
      }),
    );
  });

  it("skips when already local active (restart path)", () => {
    assert.doesNotThrow(() =>
      refuseDualBodyIfPresentElsewhere({
        agent: { ...base, status: "running" },
        presenceLookup: { [base.pubkey]: "online" },
      }),
    );
  });

  it("allowDualBody bypass", () => {
    assert.doesNotThrow(() =>
      refuseDualBodyIfPresentElsewhere({
        agent: base,
        presenceLookup: { [base.pubkey]: "online" },
        allowDualBody: true,
      }),
    );
  });
});
