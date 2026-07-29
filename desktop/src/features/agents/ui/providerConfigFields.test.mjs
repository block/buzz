import assert from "node:assert/strict";
import test from "node:test";

import {
  PROVIDER_CONFIG_OTHER_VALUE,
  providerConfigChoiceOptions,
  providerConfigChoices,
  providerConfigSelection,
  usesProviderConfigFreeText,
} from "./ProviderConfigFields.tsx";

// ── Reading the provider's suggestions ────────────────────────────────────

test("a property with no oneOf offers no choices", () => {
  assert.equal(providerConfigChoices({ type: "string" }), null);
  assert.equal(providerConfigChoices({ oneOf: "vps-prod" }), null);
});

test("oneOf entries become choices, titled when the provider titled them", () => {
  assert.deepEqual(
    providerConfigChoices({
      oneOf: [
        { const: "vps-prod", title: "vps-prod — linux · online" },
        { const: "laptop" },
      ],
    }),
    [
      { label: "vps-prod — linux · online", value: "vps-prod" },
      { label: "laptop", value: "laptop" },
    ],
  );
});

// The decoration is best-effort provider output, not validated input: one bad
// entry must not cost the user the rest of the list.
test("unusable oneOf entries are skipped, not fatal", () => {
  assert.deepEqual(
    providerConfigChoices({
      oneOf: [
        null,
        "vps-prod",
        { title: "no const" },
        { const: "" },
        { const: 7, title: "not a string" },
        { const: "laptop", title: "   " },
      ],
    }),
    [{ label: "laptop", value: "laptop" }],
  );
});

// null, not [], so the caller renders the unchanged Input rather than a
// dropdown whose only option is "Other…".
test("a oneOf with nothing usable in it reads as no choices at all", () => {
  assert.equal(providerConfigChoices({ oneOf: [] }), null);
  assert.equal(providerConfigChoices({ oneOf: [{ title: "x" }] }), null);
});

// ── Choosing between the dropdown and the text field ──────────────────────

const choices = [
  { label: "vps-prod", value: "vps-prod" },
  { label: "laptop", value: "laptop" },
];

test("an undecorated property is always free text", () => {
  assert.equal(
    usesProviderConfigFreeText({
      choices: null,
      explicitlyOther: false,
      value: "vps-prod",
    }),
    true,
  );
});

test("a listed value answers with the dropdown", () => {
  assert.equal(
    usesProviderConfigFreeText({
      choices,
      explicitlyOther: false,
      value: "laptop",
    }),
    false,
  );
});

test("an empty value answers with the dropdown", () => {
  assert.equal(
    usesProviderConfigFreeText({ choices, explicitlyOther: false, value: "" }),
    false,
  );
});

test("asking for Other opens the text field even on a listed value", () => {
  assert.equal(
    usesProviderConfigFreeText({
      choices,
      explicitlyOther: true,
      value: "laptop",
    }),
    true,
  );
});

// A value the list does not contain — carried over from before the provider
// offered a list, or a host that has since left the tailnet — must stay
// visible and editable instead of reading as "nothing selected".
test("an unlisted value stays editable without being asked", () => {
  assert.equal(
    usesProviderConfigFreeText({
      choices,
      explicitlyOther: false,
      value: "root@10.0.0.4",
    }),
    true,
  );
});

// ── Picking an option ─────────────────────────────────────────────────────

test("the escape hatch is offered last, after the provider's suggestions", () => {
  assert.deepEqual(providerConfigChoiceOptions(choices), [
    { label: "vps-prod", value: "vps-prod" },
    { label: "laptop", value: "laptop" },
    { label: "Other…", value: PROVIDER_CONFIG_OTHER_VALUE },
  ]);
});

// The common edit is a listed host with a different user or a suffix, so the
// text field must open ON the current value rather than blank.
test("picking Other keeps the value and opens free text", () => {
  assert.deepEqual(
    providerConfigSelection({
      picked: PROVIDER_CONFIG_OTHER_VALUE,
      value: "vps-prod",
    }),
    { explicitlyOther: true, value: "vps-prod" },
  );
});

test("picking a suggestion adopts it and drops the free-text override", () => {
  assert.deepEqual(
    providerConfigSelection({ picked: "laptop", value: "root@10.0.0.4" }),
    { explicitlyOther: false, value: "laptop" },
  );
});

// The full round trip a user actually takes: suggestion → Other → typed a
// custom host → back to a suggestion. The override must not survive the
// return, or the text box stays open under a dropdown that reads as answered.
test("a round trip through free text and back leaves no stuck override", () => {
  const toOther = providerConfigSelection({
    picked: PROVIDER_CONFIG_OTHER_VALUE,
    value: "vps-prod",
  });
  assert.equal(
    usesProviderConfigFreeText({
      choices,
      explicitlyOther: toOther.explicitlyOther,
      value: "root@10.0.0.4",
    }),
    true,
  );

  const backToListed = providerConfigSelection({
    picked: "laptop",
    value: "root@10.0.0.4",
  });
  assert.equal(
    usesProviderConfigFreeText({
      choices,
      explicitlyOther: backToListed.explicitlyOther,
      value: backToListed.value,
    }),
    false,
  );
});
