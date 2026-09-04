import assert from "node:assert/strict";
import test from "node:test";

import {
  EFFORT_DEFAULT_DROPDOWN_VALUE,
  effortPickerState,
  effortSelectionToPersistedValue,
} from "./effortPicker.ts";

const localBackend = { type: "local" };
const providerBackend = { type: "provider", id: "openai", config: {} };
const options = [
  { value: "low", displayName: "Low" },
  { value: "high", displayName: "High" },
];

test("effort picker renders for a local backend with a discovered configId", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: null,
  });
  assert.equal(state.visible, true);
});

test("effort picker is hidden for a provider backend even when a configId exists", () => {
  const state = effortPickerState({
    backend: providerBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: "high",
  });
  assert.equal(state.visible, false);
});

test("effort picker is hidden for a local backend without a discovered configId", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: undefined,
    effortOptions: undefined,
    currentEffort: null,
  });
  assert.equal(state.visible, false);
});

test("options lead with the adapter-default sentinel then adapter values", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
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
    effortConfigId: "thought_level",
    effortOptions: [{ value: "medium" }],
    currentEffort: null,
  });
  assert.deepEqual(state.options[1], { label: "medium", value: "medium" });
});

test("current effort preselects the matching option", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: "high",
  });
  assert.equal(state.selectValue, "high");
});

test("an unknown current effort falls back to the adapter-default sentinel", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
    effortOptions: options,
    currentEffort: "extreme",
  });
  assert.equal(state.selectValue, EFFORT_DEFAULT_DROPDOWN_VALUE);
});

test("a null current effort selects the adapter-default sentinel", () => {
  const state = effortPickerState({
    backend: localBackend,
    effortConfigId: "thought_level",
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
