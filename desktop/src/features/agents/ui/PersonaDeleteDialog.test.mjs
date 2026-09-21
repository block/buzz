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

// Boot i18n for the assertions above: Node defines a global `navigator`, so language
// detection must be pinned to English before initializeI18n() reads it.
import { initializeI18n } from "@/i18n";

Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});
initializeI18n();
