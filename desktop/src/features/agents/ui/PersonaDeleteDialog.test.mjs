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

// Provider-hosted instances are removed from the provider as part of the
// cascade — destructive beyond this machine, so the dialog must say so before
// the confirm rather than promising a purely local delete.

test("provider-hosted instances are disclosed (singular)", () => {
  const copy = personaDeleteDescription(persona, 2, 1);
  assert.match(copy, /1 of them is hosted by a provider/);
  assert.match(copy, /removed from that provider/);
});

test("provider-hosted instances are disclosed (plural)", () => {
  const copy = personaDeleteDescription(persona, 3, 2);
  assert.match(copy, /2 of them are hosted by a provider/);
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
