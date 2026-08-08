import assert from "node:assert/strict";
import test from "node:test";

import { personaDeleteDescription } from "./PersonaDeleteDialog.tsx";

// Regression guard for the persona-cascade consent copy: deleting a persona
// with instances also archives each instance's identity on the relay
// (NIP-IA 9035), a durable externally visible side effect. The confirmation
// dialog must disclose it before the destructive confirm, exactly like the
// direct agent-delete dialog does.

const persona = { displayName: "Scout" };

test("cascade delete discloses relay archival (plural)", () => {
  const copy = personaDeleteDescription(persona, 3);
  assert.match(copy, /deletes 3 agent instances/);
  assert.match(copy, /archives their identities on the relay/);
});

test("cascade delete discloses relay archival (singular)", () => {
  const copy = personaDeleteDescription(persona, 1);
  assert.match(copy, /deletes 1 agent instance /);
  assert.match(copy, /archives its identity on the relay/);
});

test("no instances → no archival claim (nothing is archived)", () => {
  const copy = personaDeleteDescription(persona, 0);
  assert.equal(copy, "Delete Scout.");
  assert.doesNotMatch(copy, /archiv/i);
});

test("null persona keeps the generic fallback", () => {
  assert.equal(personaDeleteDescription(null, 2), "Delete this agent.");
});

// Provider-hosted instances are NOT torn down by this cascade — delete_managed_agent
// never contacts the provider, and force_remote_delete only bypasses the backend's
// orphan guard. The dialog must say the deployment survives, because the failure it
// prevents is a user believing their billed infrastructure was removed when it wasn't.

test("provider-hosted instances are disclosed as surviving (singular)", () => {
  const copy = personaDeleteDescription(persona, 2, 1);
  assert.match(copy, /1 of them is hosted by a provider/);
  assert.match(copy, /not torn down/);
  assert.match(copy, /until you remove it at the provider/);
});

test("provider-hosted instances are disclosed as surviving (plural)", () => {
  const copy = personaDeleteDescription(persona, 3, 2);
  assert.match(copy, /2 of them are hosted by a provider/);
  assert.match(copy, /not torn down/);
});

test("never claims the provider deployment is removed", () => {
  // Regression guard: an earlier draft promised "will also be removed from that
  // provider", which contradicts the orphan-warning confirm shown moments later
  // in the same flow and would hide ongoing provider spend.
  for (const count of [1, 2, 5]) {
    const copy = personaDeleteDescription(persona, count + 1, count);
    assert.doesNotMatch(copy, /removed from that provider/);
    assert.doesNotMatch(copy, /will also be removed/);
  }
});

test("no provider instances → no provider claim", () => {
  const copy = personaDeleteDescription(persona, 2, 0);
  assert.doesNotMatch(copy, /provider/i);
});

test("provider disclosure defaults off for existing callers", () => {
  assert.equal(
    personaDeleteDescription(persona, 2),
    personaDeleteDescription(persona, 2, 0),
  );
});
