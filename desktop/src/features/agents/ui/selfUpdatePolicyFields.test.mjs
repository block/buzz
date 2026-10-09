import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { SelfUpdatePolicyFields } from "./SelfUpdatePolicyFields.tsx";
import {
  SELF_UPDATE_FIELDS,
  describeSelfUpdate,
  selfUpdateFieldLabel,
  toggleSelfUpdateField,
} from "../agentManagement.ts";

// ── Labels and the audit toast ───────────────────────────────────────────────

test("every self-updatable field has a human label", () => {
  assert.deepEqual(SELF_UPDATE_FIELDS.map(selfUpdateFieldLabel), [
    "system prompt",
    "model",
    "display name",
  ]);
});

test("the audit toast names the agent and every field it changed", () => {
  assert.equal(
    describeSelfUpdate("Scout", ["system_prompt"]),
    "Scout updated its own system prompt",
  );
  assert.equal(
    describeSelfUpdate("Scout", ["system_prompt", "model", "display_name"]),
    "Scout updated its own system prompt, model and display name",
  );
  assert.equal(
    describeSelfUpdate("Scout", []),
    "Scout updated its own definition",
  );
});

test("toggling keeps canonical order and never duplicates a field", () => {
  assert.deepEqual(toggleSelfUpdateField([], "model", true), ["model"]);
  assert.deepEqual(toggleSelfUpdateField(["model"], "system_prompt", true), [
    "system_prompt",
    "model",
  ]);
  assert.deepEqual(toggleSelfUpdateField(["model"], "model", true), ["model"]);
  assert.deepEqual(
    toggleSelfUpdateField(["system_prompt", "model"], "model", false),
    ["system_prompt"],
  );
});

// ── Policy checkboxes ────────────────────────────────────────────────────────

function render(value, disabled = false) {
  return renderToStaticMarkup(
    React.createElement(SelfUpdatePolicyFields, {
      disabled,
      onChange: () => {},
      value,
    }),
  );
}

test("renders one labelled checkbox per field, unchecked by default", () => {
  const html = render([]);
  for (const field of SELF_UPDATE_FIELDS) {
    const id = `edit-agent-self-update-${field}`;
    assert.match(html, new RegExp(`<input[^>]*id="${id}"[^>]*type="checkbox"`));
    assert.match(html, new RegExp(`<label[^>]*for="${id}"`));
  }
  assert.equal((html.match(/checked=""/g) ?? []).length, 0);
  assert.match(html, /Allow this agent to update its own/);
  assert.match(html, /opens a review form; nothing changes until you save it/);
});

function checkbox(html, field) {
  const match = html.match(
    new RegExp(`<input[^>]*id="edit-agent-self-update-${field}"[^>]*/>`),
  );
  assert.ok(match, `checkbox for ${field} must render`);
  return match[0];
}

test("checked fields render checked and the helper text says review is skipped", () => {
  const html = render(["model", "display_name"]);
  assert.match(checkbox(html, "model"), /checked=""/);
  assert.match(checkbox(html, "display_name"), /checked=""/);
  assert.doesNotMatch(checkbox(html, "system_prompt"), /checked=""/);
  assert.match(html, /applied without review/);
});

test("disabled propagates to every checkbox", () => {
  const html = render(["model"], true);
  assert.equal((html.match(/disabled=""/g) ?? []).length, 3);
});
