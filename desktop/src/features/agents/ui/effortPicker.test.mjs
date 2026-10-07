import assert from "node:assert/strict";
import test from "node:test";

import {
  EFFORT_DEFAULT_DROPDOWN_VALUE,
  EFFORT_LEVELS_UNKNOWN,
  effortPickerState,
  effortChoices,
  effortSelectionToPersistedValue,
  isSavableEffort,
} from "./effortPicker.ts";

const localBackend = { type: "local" };
const providerBackend = { type: "provider", id: "openai", config: {} };
const options = [
  { value: "low", displayName: "Low" },
  { value: "high", displayName: "High" },
];

test("effort picker renders for a local backend with known levels", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: options,
    currentEffort: null,
  });
  assert.equal(state.visible, true);
});

test("effort picker is hidden for a provider backend even with known levels", () => {
  const state = effortPickerState({
    backend: providerBackend,
    effortOptions: options,
    currentEffort: "high",
  });
  assert.equal(state.visible, false);
});

test("effort picker is hidden for a local backend whose model offers no levels", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: undefined,
    currentEffort: null,
  });
  assert.equal(state.visible, false);
});

test("options lead with the adapter-default sentinel then adapter values", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: options,
    currentEffort: null,
  });
  assert.deepEqual(state.options, [
    { label: "Adapter default", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    { label: "Low", value: "low" },
    { label: "High", value: "high" },
  ]);
});

test("option label falls back to the raw value when displayName is absent", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: [{ value: "medium" }],
    currentEffort: null,
  });
  assert.deepEqual(state.options[1], { label: "medium", value: "medium" });
});

test("current effort preselects the matching option", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: options,
    currentEffort: "high",
  });
  assert.equal(state.selectValue, "high");
});

test("an unknown current effort falls back to the adapter-default sentinel", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: options,
    currentEffort: "extreme",
  });
  assert.equal(state.selectValue, EFFORT_DEFAULT_DROPDOWN_VALUE);
});

test("a null current effort selects the adapter-default sentinel", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: options,
    currentEffort: null,
  });
  assert.equal(state.selectValue, EFFORT_DEFAULT_DROPDOWN_VALUE);
});

test("the sentinel selection persists as null (clear to adapter default)", () => {
  assert.equal(
    effortSelectionToPersistedValue(EFFORT_DEFAULT_DROPDOWN_VALUE),
    null,
  );
});

test("a concrete selection persists as its explicit effort level", () => {
  assert.equal(effortSelectionToPersistedValue("high"), "high");
});

const values = (choices) => choices?.map((choice) => choice.value);
const storedOpusSession = {
  effortConfigId: "thought_level",
  effortOptions: [{ value: "low" }, { value: "max" }],
};

test("effortChoices_blankModel_usesAdapterDefaultOpus1mAlias", () => {
  const choices = effortChoices({
    runtimeId: "claude",
    models: ["", null, "opus[1m]"],
    sessionApplies: false,
  });
  assert.deepEqual(values(choices), ["low", "medium", "high"]);
});

test("effortChoices_haikuGlobalOverride_beatsAdapterDefault", () => {
  const choices = effortChoices({
    runtimeId: "claude",
    models: ["", "claude-haiku-4-5", "opus[1m]"],
    sessionApplies: false,
  });
  assert.equal(choices, undefined);
});

test("effortChoices_explicitModel_beatsGlobalOverride", () => {
  const choices = effortChoices({
    runtimeId: "claude",
    models: ["claude-opus-4-8", "claude-haiku-4-5", null],
    sessionApplies: false,
  });
  assert.deepEqual(values(choices), ["low", "medium", "high", "xhigh", "max"]);
});

test("effortChoices_claudeStoredSession_neverTrusted", () => {
  // The stored surface does not say which model the session ran.
  const choices = effortChoices({
    runtimeId: "claude",
    models: ["claude-haiku-4-5"],
    sessionApplies: true,
    session: storedOpusSession,
  });
  assert.equal(choices, undefined);
});

test("effortChoices_codexStoredSession_winsWhileRuntimeUnchanged", () => {
  const choices = effortChoices({
    runtimeId: "codex",
    models: ["gpt-5.5"],
    sessionApplies: true,
    session: storedOpusSession,
  });
  assert.deepEqual(values(choices), ["low", "max"]);
});

test("effortChoices_nonClaudeRuntime_keepsNativeOnlyBehavior", () => {
  const choices = effortChoices({
    runtimeId: "codex",
    models: ["opus[1m]"],
    sessionApplies: false,
  });
  assert.equal(choices, undefined);
});

test("effortChoices_discoveryPendingOrFailed_reportsUnknownModel", () => {
  // The hook returns agentDefaultModel: null until its keyed response lands.
  const choices = effortChoices({
    runtimeId: "claude",
    models: ["", null, null],
    sessionApplies: false,
  });
  assert.equal(choices, EFFORT_LEVELS_UNKNOWN);
});

for (const alias of ["default", "opusplan", "default[1m]", "OpusPlan[1m]"]) {
  test(`effortChoices_claudeAlias_${alias}_offersAliasLevels`, () => {
    const choices = effortChoices({
      runtimeId: "claude",
      models: [alias],
      sessionApplies: false,
    });
    assert.deepEqual(values(choices), ["low", "medium", "high"]);
  });
}

test("effortChoices_unrecognizedClaudeModel_offersNone", () => {
  // The manifest's unknown fallback is also what Haiku resolves to, so an
  // unrecognized id cannot be told apart from a model without levels.
  const choices = effortChoices({
    runtimeId: "claude",
    models: ["claude-future-9"],
    sessionApplies: false,
  });
  assert.equal(choices, undefined);
});

test("isSavableEffort_rejectsLevelTheModelDoesNotOffer", () => {
  assert.equal(isSavableEffort("high", undefined), false);
  assert.equal(isSavableEffort("max", [{ value: "high" }]), false);
  assert.equal(isSavableEffort("high", [{ value: "high" }]), true);
  assert.equal(isSavableEffort(null, undefined), true);
});

test("isSavableEffort_unknownModel_keepsThePick", () => {
  assert.equal(isSavableEffort("high", EFFORT_LEVELS_UNKNOWN), true);
});

test("effortPickerState_unknownModelNothingStored_hidesThePicker", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: EFFORT_LEVELS_UNKNOWN,
    currentEffort: "high",
  });
  assert.equal(state.visible, false);
});

test("effortPickerState_unknownModelWithStoredLevel_showsItAsUnknown", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: EFFORT_LEVELS_UNKNOWN,
    currentEffort: "max",
    storedEffort: "max",
  });
  assert.equal(state.visible, true);
  assert.equal(state.note, "unknownModel");
  assert.deepEqual(state.options, [
    { label: "Adapter default", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    { label: "max", value: "max" },
  ]);
});

test("effortPickerState_unknownModelWithStoredLevel_showsThePickSaveKeeps", () => {
  // isSavableEffort keeps any pick while the model is unknown.
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: EFFORT_LEVELS_UNKNOWN,
    currentEffort: "high",
    storedEffort: "max",
  });
  assert.equal(state.selectValue, "high");
  assert.equal(state.note, "unknownModel");
  assert.deepEqual(
    state.options.map((option) => option.value),
    [EFFORT_DEFAULT_DROPDOWN_VALUE, "max", "high"],
  );
});

test("effortPickerState_storedLevelTheModelDoesNotList_staysSelectable", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: undefined,
    currentEffort: "max",
    storedEffort: "max",
  });
  assert.equal(state.visible, true);
  assert.equal(state.note, "unlisted");
  assert.equal(state.selectValue, "max");
  assert.deepEqual(state.options, [
    { label: "Adapter default", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    { label: "max", value: "max" },
  ]);
});

test("effortPickerState_clearedUnlistedStoredLevel_staysOfferedButUnselected", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: options,
    currentEffort: null,
    storedEffort: "max",
  });
  assert.equal(state.visible, true);
  assert.equal(state.note, null);
  assert.equal(state.selectValue, EFFORT_DEFAULT_DROPDOWN_VALUE);
  assert.equal(state.options.at(-1).value, "max");
});

test("effortPickerState_listedSelection_hasNoNote", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortOptions: options,
    currentEffort: "high",
    storedEffort: "max",
  });
  assert.equal(state.selectValue, "high");
  assert.equal(state.note, null);
});
