import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_ACP_COMMAND_VALUE,
  acpCommandPickerState,
} from "./acpCommandPicker.ts";

const candidate = {
  command: "buzz-janet-acp",
  binaryPath: "/bin/buzz-janet-acp",
};

test("stock and discovered commands select discovered options", () => {
  const defaultState = acpCommandPickerState("buzz-acp", [candidate]);
  assert.equal(defaultState.selectValue, "buzz-acp");
  assert.deepEqual(defaultState.options, [
    { label: "Buzz ACP (default)", value: "buzz-acp" },
    { label: "buzz-janet-acp", value: "buzz-janet-acp" },
  ]);

  assert.equal(
    acpCommandPickerState("buzz-janet-acp", [candidate]).selectValue,
    "buzz-janet-acp",
  );
});

test("an empty command selects the stock default", () => {
  assert.equal(
    acpCommandPickerState("", [candidate]).selectValue,
    DEFAULT_ACP_COMMAND_VALUE,
  );
});

test("a persisted unknown command is preserved as a read-only current option", () => {
  const state = acpCommandPickerState("my-acp", [candidate]);
  assert.equal(state.selectValue, "my-acp");
  assert.deepEqual(state.options.at(-1), {
    disabled: true,
    label: "my-acp (unavailable)",
    value: "my-acp",
  });
});

test("late discovery replaces the current marker without changing the command", () => {
  const before = acpCommandPickerState("buzz-janet-acp", []);
  assert.deepEqual(before.options.at(-1), {
    disabled: true,
    label: "buzz-janet-acp (unavailable)",
    value: "buzz-janet-acp",
  });

  const after = acpCommandPickerState("buzz-janet-acp", [candidate]);
  assert.equal(after.selectValue, "buzz-janet-acp");
  assert.deepEqual(after.options.at(-1), {
    label: "buzz-janet-acp",
    value: "buzz-janet-acp",
  });
});
