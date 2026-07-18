import assert from "node:assert/strict";
import test from "node:test";

import { personaSubmitBlock } from "./personaSubmitBlock.ts";

/** A fully valid, submittable form: personaSubmitBlock returns null. */
function submittable(overrides = {}) {
  return {
    isPending: false,
    isAvatarUploadPending: false,
    displayNameEmpty: false,
    isCreateMode: true,
    runtimeChosen: true,
    runtimeAvailable: true,
    createBackendBlocked: false,
    allowlistEmpty: false,
    aiConfigurationMode: "defaults",
    localModeSatisfied: true,
    localModeMissingFields: [],
    localModeMissingEnvKeys: [],
    customAiPairSatisfied: true,
    runtimeNeedsProviderSelection: true,
    customProviderEmpty: false,
    customModelEmpty: false,
    ...overrides,
  };
}

test("a valid form has no disabled reason", () => {
  assert.equal(personaSubmitBlock(submittable()), null);
});

test("missing name is reported first", () => {
  assert.equal(
    personaSubmitBlock(submittable({ displayNameEmpty: true })),
    "Enter a name for this agent.",
  );
});

test("Buzz Agent + Use AI defaults with no global provider/model names the fix", () => {
  const reason = personaSubmitBlock(
    submittable({
      aiConfigurationMode: "defaults",
      localModeSatisfied: false,
      localModeMissingFields: ["provider", "model"],
    }),
  );
  assert.match(reason, /global AI defaults are incomplete/);
  assert.match(reason, /a provider and a model/);
  assert.match(reason, /Settings → AI defaults/);
});

test("incomplete defaults also names missing credential keys", () => {
  const reason = personaSubmitBlock(
    submittable({
      localModeSatisfied: false,
      localModeMissingFields: [],
      localModeMissingEnvKeys: ["ANTHROPIC_API_KEY"],
    }),
  );
  assert.match(reason, /a value for ANTHROPIC_API_KEY/);
});

test("the reason disappears once the blocking input is corrected", () => {
  const blocked = submittable({
    localModeSatisfied: false,
    localModeMissingFields: ["provider", "model"],
  });
  assert.notEqual(personaSubmitBlock(blocked), null);
  // Correct the blocking input: defaults now resolve.
  const corrected = {
    ...blocked,
    localModeSatisfied: true,
    localModeMissingFields: [],
  };
  assert.equal(personaSubmitBlock(corrected), null);
});

test("create mode requires a chosen, available runtime", () => {
  assert.equal(
    personaSubmitBlock(submittable({ runtimeChosen: false })),
    "Choose where this agent runs.",
  );
  assert.equal(
    personaSubmitBlock(submittable({ runtimeAvailable: false })),
    "The selected runtime isn't available on this machine.",
  );
});

test("runtime gates do not apply in edit mode", () => {
  assert.equal(
    personaSubmitBlock(
      submittable({ isCreateMode: false, runtimeChosen: false }),
    ),
    null,
  );
});

test("empty allowlist is reported (create and edit)", () => {
  const reason = personaSubmitBlock(
    submittable({ isCreateMode: false, allowlistEmpty: true }),
  );
  assert.match(reason, /allowed sender/);
});

test("Customize with an empty pair but satisfied global fallback points at the pair", () => {
  const reason = personaSubmitBlock(
    submittable({
      aiConfigurationMode: "custom",
      localModeSatisfied: true,
      customAiPairSatisfied: false,
      customProviderEmpty: true,
      customModelEmpty: true,
    }),
  );
  assert.match(reason, /Select a provider and a model/);
  assert.match(reason, /Use AI defaults/);
});

test("Customize on Codex/Claude asks only for a model, never a provider", () => {
  const reason = personaSubmitBlock(
    submittable({
      aiConfigurationMode: "custom",
      customAiPairSatisfied: false,
      runtimeNeedsProviderSelection: false,
      customProviderEmpty: true,
      customModelEmpty: true,
    }),
  );
  assert.match(reason, /Select a model/);
  assert.doesNotMatch(reason, /provider/);
  assert.match(reason, /Use harness defaults/);
  assert.doesNotMatch(reason, /Use AI defaults/);
});

test("precedence: a missing name outranks incomplete AI defaults", () => {
  assert.equal(
    personaSubmitBlock(
      submittable({
        displayNameEmpty: true,
        localModeSatisfied: false,
        localModeMissingFields: ["provider", "model"],
      }),
    ),
    "Enter a name for this agent.",
  );
});

test("in-flight save/upload shows no reason (the button label communicates it)", () => {
  assert.equal(
    personaSubmitBlock(
      submittable({ isPending: true, displayNameEmpty: true }),
    ),
    null,
  );
  assert.equal(
    personaSubmitBlock(submittable({ isAvatarUploadPending: true })),
    null,
  );
});
