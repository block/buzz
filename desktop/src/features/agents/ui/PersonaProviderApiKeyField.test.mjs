/**
 * Behavioral tests for PersonaProviderApiKeyField.
 *
 * Tests the rendering invariants that matter for the disambiguation story:
 * - semantic label is present in the rendered output
 * - envVarName hint is rendered when the prop is present
 * - hint id is wired to the input via aria-describedby
 * - hint is absent when envVarName is omitted
 * - two simultaneous instances produce unique IDs (no duplicate-ID collision
 *   in the nested AgentInstanceEditDialog + AgentDefaultsDialog path)
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField.tsx";

function makeProps(overrides = {}) {
  return {
    disabled: false,
    isInherited: false,
    inheritedLabel: "Set in global defaults",
    isRequired: false,
    label: "OpenAI Runtime API Key",
    onValueChange: () => {},
    value: "",
    ...overrides,
  };
}

/** Extract the value of the first attribute matching `name="..."` in html. */
function extractAttr(html, attrName) {
  const re = new RegExp(`${attrName}="([^"]+)"`);
  const m = re.exec(html);
  return m ? m[1] : null;
}

/** Extract ALL values of an attribute from html, in document order. */
function extractAllAttrs(html, attrName) {
  const re = new RegExp(`${attrName}="([^"]+)"`, "g");
  return Array.from(html.matchAll(re), (m) => m[1]);
}

test("PersonaProviderApiKeyField_renders_semantic_label", () => {
  const html = renderToStaticMarkup(
    React.createElement(PersonaProviderApiKeyField, makeProps()),
  );
  assert.ok(
    html.includes("OpenAI Runtime API Key"),
    "semantic label must appear in rendered output",
  );
});

test("PersonaProviderApiKeyField_renders_env_var_hint_when_envVarName_present", () => {
  const html = renderToStaticMarkup(
    React.createElement(
      PersonaProviderApiKeyField,
      makeProps({ envVarName: "OPENAI_COMPAT_API_KEY" }),
    ),
  );
  assert.ok(
    html.includes("OPENAI_COMPAT_API_KEY"),
    "env-var hint must appear when envVarName is provided",
  );
});

test("PersonaProviderApiKeyField_wires_hint_id_via_aria_describedby", () => {
  const html = renderToStaticMarkup(
    React.createElement(
      PersonaProviderApiKeyField,
      makeProps({ envVarName: "OPENAI_COMPAT_API_KEY" }),
    ),
  );
  // Extract the dynamically-generated hint id from the rendered paragraph.
  const hintId = extractAttr(html, "id");
  assert.ok(hintId, "hint paragraph must have an id");
  assert.ok(
    hintId.startsWith("persona-provider-api-key-hint-"),
    `hint id must follow the expected prefix, got: ${hintId}`,
  );
  // The input's aria-describedby must point at the same id.
  const describedBy = extractAttr(html, "aria-describedby");
  assert.equal(
    describedBy,
    hintId,
    "input aria-describedby must reference the hint's id",
  );
});

test("PersonaProviderApiKeyField_omits_hint_when_envVarName_absent", () => {
  const html = renderToStaticMarkup(
    React.createElement(PersonaProviderApiKeyField, makeProps()),
  );
  assert.ok(
    !html.includes("aria-describedby"),
    "no aria-describedby when envVarName is omitted",
  );
  assert.ok(
    !html.includes("persona-provider-api-key-hint"),
    "hint id must not appear when envVarName is omitted",
  );
});

test("PersonaProviderApiKeyField_two_instances_have_unique_ids_and_each_aria_describedby_resolves_to_own_hint", () => {
  // Render BOTH instances in a single renderToStaticMarkup call — this
  // mirrors the real nested-dialog DOM where AgentInstanceEditDialog's
  // credential field and the nested AgentDefaultsDialog's field are
  // simultaneously mounted under the same React root. A shared root is what
  // makes React.useId() guarantee uniqueness; two separate renderToStaticMarkup
  // calls each reset the counter and would produce the same ID.
  const combined = renderToStaticMarkup(
    React.createElement(
      React.Fragment,
      null,
      React.createElement(
        PersonaProviderApiKeyField,
        makeProps({
          label: "Anthropic API Key",
          envVarName: "ANTHROPIC_API_KEY",
        }),
      ),
      React.createElement(
        PersonaProviderApiKeyField,
        makeProps({
          label: "OpenAI Runtime API Key",
          envVarName: "OPENAI_COMPAT_API_KEY",
        }),
      ),
    ),
  );

  // Two hint paragraph ids must be present and distinct.
  const allHintIds = extractAllAttrs(combined, "id").filter((id) =>
    id.startsWith("persona-provider-api-key-hint-"),
  );
  assert.equal(allHintIds.length, 2, "exactly two hint ids must be present");
  const [hintIdA, hintIdB] = allHintIds;
  assert.notEqual(hintIdA, hintIdB, "two instances must not share a hint id");

  // Each input's aria-describedby must match its own hint id (same order).
  const allDescribedBy = extractAllAttrs(combined, "aria-describedby");
  assert.equal(
    allDescribedBy.length,
    2,
    "exactly two aria-describedby attributes must be present",
  );
  assert.equal(
    allDescribedBy[0],
    hintIdA,
    "instance A: aria-describedby must reference instance A's own hint",
  );
  assert.equal(
    allDescribedBy[1],
    hintIdB,
    "instance B: aria-describedby must reference instance B's own hint",
  );

  // Confirm each instance names its own env var in the rendered output.
  assert.ok(
    combined.includes("ANTHROPIC_API_KEY"),
    "combined output must name ANTHROPIC_API_KEY",
  );
  assert.ok(
    combined.includes("OPENAI_COMPAT_API_KEY"),
    "combined output must name OPENAI_COMPAT_API_KEY",
  );
});
