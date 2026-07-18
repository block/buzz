import assert from "node:assert/strict";
import test from "node:test";

import { runtimeCanBeSelected } from "./onboardingRuntimeSelection.ts";

function runtime(id, availability, status) {
  return { id, availability, authStatus: { status } };
}

test("Claude and Codex require available authenticated CLIs", () => {
  for (const id of ["claude", "codex"]) {
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "logged_in")),
      true,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "not_applicable")),
      true,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "logged_out")),
      false,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "config_invalid")),
      false,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "unknown")),
      false,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "not_installed", "logged_in")),
      false,
    );
  }
});

test("Buzz Agent and Goose remain selectable when available", () => {
  for (const id of ["buzz-agent", "goose"]) {
    assert.equal(
      runtimeCanBeSelected(runtime(id, "available", "not_applicable")),
      true,
    );
    assert.equal(
      runtimeCanBeSelected(runtime(id, "not_installed", "not_applicable")),
      false,
    );
  }
});

test("unknown runtimes are not onboarding choices", () => {
  assert.equal(
    runtimeCanBeSelected(runtime("custom", "available", "logged_in")),
    false,
  );
});
