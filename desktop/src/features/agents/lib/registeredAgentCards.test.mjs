import assert from "node:assert/strict";
import test from "node:test";

import {
  dedupeRegisteredAgentsAgainstManaged,
  resolveRegisteredAgentDisplay,
  registeredAgentRoleSummary,
  visibleRegisteredAgentReferences,
} from "./registeredAgentCards.ts";

const PUBKEY = "a1".repeat(32);
const OTHER = "b2".repeat(32);
const reference = (pubkey, label, roleSummary = null) => ({
  pubkey,
  label,
  roleSummary,
  createdAt: "",
  updatedAt: "",
});

test("display prefers profile display name, then label, then short key name", () => {
  assert.equal(
    resolveRegisteredAgentDisplay({
      reference: reference(PUBKEY, "Stored Label", "reviewer"),
      profile: { displayName: "Relay Profile", avatarUrl: null },
    }).label,
    "Relay Profile",
  );
  assert.equal(
    resolveRegisteredAgentDisplay({
      reference: reference(PUBKEY, "Stored Label", "reviewer"),
      profile: { displayName: " ", avatarUrl: null },
    }).label,
    "Stored Label",
  );
  assert.equal(
    resolveRegisteredAgentDisplay({
      reference: reference(PUBKEY, null),
      profile: null,
    }).label,
    `${PUBKEY.slice(0, 8)}…${PUBKEY.slice(-4)}`,
  );
});

test("role summary is always an externally managed suffix", () => {
  assert.equal(
    registeredAgentRoleSummary("reviewer"),
    "reviewer · Externally managed",
  );
  assert.equal(registeredAgentRoleSummary(null), "Externally managed");
});

test("registered references are deduped against managed pubkeys", () => {
  const references = [
    reference(PUBKEY.toUpperCase(), "external"),
    reference(OTHER, "visible"),
  ];
  assert.deepEqual(
    dedupeRegisteredAgentsAgainstManaged(references, [{ pubkey: PUBKEY }]),
    [reference(OTHER, "visible")],
  );
});

test("registered-reference errors fail closed over stale cached data", () => {
  const staleReferences = [reference(OTHER, "stale")];
  assert.deepEqual(
    visibleRegisteredAgentReferences(
      staleReferences,
      [],
      new Error("reference store invalid"),
    ),
    [],
  );
  assert.deepEqual(
    visibleRegisteredAgentReferences(staleReferences, [], null),
    staleReferences,
  );
});
