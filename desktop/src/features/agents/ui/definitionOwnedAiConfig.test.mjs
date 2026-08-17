/**
 * Contract tests for definition-owned AI config on the instance edit form.
 *
 * #1968 made the linked definition authoritative for model, LLM provider, and
 * system prompt: `resolve_effective_config` reads them from the definition and
 * never consults the instance record. That commit changed the write path and
 * left the instance form's Model and LLM provider dropdowns live, so editing
 * them reported a successful save and changed nothing — the user had to find
 * the definition dialog and set the same field a second time.
 *
 * The fix is one predicate feeding both seams. These tests pin that: if a
 * future change re-derives either the omission or the control state from its
 * own `linkedPersona != null` check, the two can drift apart again and this
 * file fails.
 */

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { definitionOwnsAiConfig } from "./personaRuntimeModel.ts";

const dialogSource = await readFile(
  new URL("./AgentInstanceEditDialog.tsx", import.meta.url),
  "utf8",
);
const modelFieldSource = await readFile(
  new URL("./EditAgentModelField.tsx", import.meta.url),
  "utf8",
);

/** JSX wraps wherever the formatter decides; match on collapsed whitespace. */
const collapsedDialog = dialogSource.replace(/\s+/g, " ");

test("a linked definition owns the AI config", () => {
  assert.equal(definitionOwnsAiConfig({ id: "persona-1" }), true);
});

test("a definition-less instance owns its own AI config", () => {
  // Both absent forms: no personaId at all, and a personaId whose definition
  // has not resolved (or no longer exists — an orphan still edits locally).
  assert.equal(definitionOwnsAiConfig(null), false);
  assert.equal(definitionOwnsAiConfig(undefined), false);
});

test("the submit path omits all three definition-owned fields", () => {
  for (const field of ["systemPrompt", "model", "provider"]) {
    assert.match(
      collapsedDialog,
      new RegExp(`${field}: aiConfigIsDefinitionOwned \\? undefined`),
      `${field} must be omitted from UpdateManagedAgentInput when the definition owns it`,
    );
  }
});

test("no seam re-derives definition ownership on its own", () => {
  // One call, at the top of the component. Everything else reads the result.
  const derivations = dialogSource.match(/definitionOwnsAiConfig\(/g) ?? [];
  assert.equal(derivations.length, 1);
  assert.doesNotMatch(dialogSource, /linkedPersona != null/);
});

test("the shared disabled flag is the one the submit path reads", () => {
  assert.match(
    collapsedDialog,
    /const aiFieldsDisabled = updateMutation\.isPending \|\| aiConfigIsDefinitionOwned;/,
  );
});

test("every control for a definition-owned field is disabled with it", () => {
  // The dropdown and its custom-value escape hatch, for both fields. Missing
  // one of the four leaves a live control over a value the save discards.
  // The model pair lives in EditAgentModelField, which the dialog hands the
  // same flag; the assertion below pins that hand-off.
  for (const [source, id, flag] of [
    [collapsedDialog, "edit-agent-llm-provider", "aiFieldsDisabled"],
    [collapsedDialog, "edit-agent-custom-provider", "aiFieldsDisabled"],
    [modelFieldSource, "edit-agent-model", "disabled"],
    [modelFieldSource, "edit-agent-custom-model", "disabled"],
  ]) {
    const controlAt = source.indexOf(`id="${id}"`);
    assert.ok(controlAt > 0, `${id} not found`);
    // `disabled` precedes `id` in every control in these files (props are
    // alphabetized), so scan back to the start of the element.
    const elementAt = source.lastIndexOf("<", controlAt);
    assert.match(
      source.slice(elementAt, controlAt),
      new RegExp(`disabled=\\{[^}]*${flag}`),
      `${id} must be disabled when the definition owns the value`,
    );
  }

  assert.match(
    collapsedDialog,
    /<EditAgentModelField disabled=\{aiFieldsDisabled\}/,
  );
});

test("a definition-owned provider never blocks Save", () => {
  // Changing harness re-derives the provider draft and can blank it. With the
  // provider control read-only that would be an unrepairable disabled Save on
  // a value this form does not send at all.
  assert.match(
    collapsedDialog,
    /const providerValid = aiConfigIsDefinitionOwned \|\| isEditAgentProviderSaveValid\(\{/,
  );
});

test("the read-only summary offers a route to the definition", () => {
  // A disabled control with no way to reach the real one is the same dead end
  // in a different costume.
  assert.match(
    collapsedDialog,
    /onEditDefinition=\{ aiConfigIsDefinitionOwned \? handleEditLinkedPersona : undefined \}/,
  );
});
