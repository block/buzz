import assert from "node:assert/strict";
import test from "node:test";

import {
  definitionCreateDialogState,
  intentForStartToggle,
  resolveCreateIntent,
} from "./agentCreateIntent.ts";
import { createPersonaDialogState } from "./personaDialogState.ts";

test("resolveCreateIntent defaults to quick-start for un-migrated callers", () => {
  // PersonaDialog's duplicate path calls handleSubmit without an intent until
  // B3 migrates it; the default must preserve today's create-then-start
  // behavior or duplicate silently becomes definition-only.
  assert.equal(resolveCreateIntent(undefined), "definition_start");
});

test("resolveCreateIntent passes explicit intents through", () => {
  assert.equal(resolveCreateIntent("definition"), "definition");
  assert.equal(resolveCreateIntent("definition_start"), "definition_start");
  assert.equal(resolveCreateIntent("instance"), "instance");
});

test("intentForStartToggle maps the toggle to definition-family intents", () => {
  assert.equal(intentForStartToggle(true), "definition_start");
  assert.equal(intentForStartToggle(false), "definition");
});

test("toggle-on dialog copy is exactly the legacy create copy", () => {
  assert.deepEqual(
    definitionCreateDialogState(true),
    createPersonaDialogState(),
  );
});

test("toggle-off dialog copy differs only in description", () => {
  const legacy = createPersonaDialogState();
  const definitionOnly = definitionCreateDialogState(false);
  assert.equal(definitionOnly.title, legacy.title);
  assert.equal(definitionOnly.submitLabel, legacy.submitLabel);
  assert.deepEqual(definitionOnly.initialValues, legacy.initialValues);
  assert.notEqual(definitionOnly.description, legacy.description);
  assert.match(definitionOnly.description, /without starting/);
});
