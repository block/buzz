import assert from "node:assert/strict";
import test from "node:test";

import {
  collectRuntimeWarnings,
  resolvePersonaRuntime,
} from "./resolvePersonaRuntime.ts";

function makeRuntime(id, label = `${id} label`) {
  return { id, label, command: id, avatarUrl: "" };
}

const goose = makeRuntime("goose", "Goose");
const claude = makeRuntime("claude", "Claude");
const runtimes = [goose, claude];

test("resolvePersonaRuntime — no personaRuntimeId returns defaultRuntime with no warnings", () => {
  const result = resolvePersonaRuntime(null, runtimes, goose);
  assert.deepEqual(result, {
    runtime: goose,
    warnings: [],
    isOverridden: false,
  });
});

test("resolvePersonaRuntime — undefined personaRuntimeId also returns defaultRuntime", () => {
  const result = resolvePersonaRuntime(undefined, runtimes, goose);
  assert.deepEqual(result, {
    runtime: goose,
    warnings: [],
    isOverridden: false,
  });
});

test("resolvePersonaRuntime — no personaRuntimeId and no defaultRuntime returns null with warning", () => {
  const result = resolvePersonaRuntime(null, runtimes, null);
  assert.equal(result.runtime, null);
  assert.equal(result.warnings.length, 1);
  assert.match(result.warnings[0], /No agent runtimes are available/);
  assert.equal(result.isOverridden, false);
});

test("resolvePersonaRuntime — matching runtime found returns matched runtime, no warnings", () => {
  const result = resolvePersonaRuntime("goose", runtimes, claude);
  assert.deepEqual(result, {
    runtime: goose,
    warnings: [],
    isOverridden: false,
  });
});

test("resolvePersonaRuntime — override=true with same runtime as default returns default, no warnings", () => {
  const result = resolvePersonaRuntime("goose", runtimes, goose, true);
  assert.deepEqual(result, {
    runtime: goose,
    warnings: [],
    isOverridden: false,
  });
});

test("resolvePersonaRuntime — override=true with different default emits override warning and returns default", () => {
  const result = resolvePersonaRuntime("goose", runtimes, claude, true);
  assert.equal(result.runtime, claude);
  assert.equal(result.warnings.length, 1);
  assert.match(result.warnings[0], /Runtime override/);
  assert.match(result.warnings[0], /Claude/);
  assert.match(result.warnings[0], /Goose/);
  assert.equal(result.isOverridden, true);
});

test("resolvePersonaRuntime — override=false returns matched runtime, ignores override flag", () => {
  const result = resolvePersonaRuntime("goose", runtimes, claude, false);
  assert.deepEqual(result, {
    runtime: goose,
    warnings: [],
    isOverridden: false,
  });
});

test("resolvePersonaRuntime — override=true but no defaultRuntime returns matched runtime, no warnings", () => {
  const result = resolvePersonaRuntime("goose", runtimes, null, true);
  assert.deepEqual(result, {
    runtime: goose,
    warnings: [],
    isOverridden: false,
  });
});

test("resolvePersonaRuntime — unrecognised runtimeId falls back to defaultRuntime with warning", () => {
  const result = resolvePersonaRuntime("unknown-rt", runtimes, goose);
  assert.equal(result.runtime, goose);
  assert.equal(result.warnings.length, 1);
  assert.match(result.warnings[0], /unknown-rt/);
  assert.match(result.warnings[0], /Goose/);
  assert.match(result.warnings[0], /not available/);
  assert.equal(result.isOverridden, true);
});

test("resolvePersonaRuntime — unrecognised runtimeId and no defaultRuntime returns null with error warning", () => {
  const result = resolvePersonaRuntime("unknown-rt", [], null);
  assert.equal(result.runtime, null);
  assert.equal(result.warnings.length, 1);
  assert.match(result.warnings[0], /unknown-rt/);
  assert.match(result.warnings[0], /no other runtimes were found/);
  assert.equal(result.isOverridden, false);
});

test("resolvePersonaRuntime — isOverridden is true when override redirects to different runtime", () => {
  const result = resolvePersonaRuntime("goose", runtimes, claude, true);
  assert.equal(result.isOverridden, true);
});

test("resolvePersonaRuntime — isOverridden is false when no override active", () => {
  const result = resolvePersonaRuntime("goose", runtimes, claude);
  assert.equal(result.isOverridden, false);
});

test("resolvePersonaRuntime — isOverridden is true when persona's runtime is unavailable and falls back", () => {
  const result = resolvePersonaRuntime("unknown-rt", runtimes, goose);
  assert.equal(result.isOverridden, true);
});

test("resolvePersonaRuntime — isOverridden is false when override selects same runtime as persona", () => {
  const result = resolvePersonaRuntime("goose", runtimes, goose, true);
  assert.equal(result.isOverridden, false);
});

test("collectRuntimeWarnings — no fallbackRuntime returns empty array regardless of personas", () => {
  const personas = [{ runtime: "goose" }, { runtime: "unknown-rt" }];
  const warnings = collectRuntimeWarnings(personas, runtimes, null);
  assert.deepEqual(warnings, []);
});

test("collectRuntimeWarnings — all personas match their runtimes returns empty array", () => {
  const personas = [{ runtime: "goose" }, { runtime: "claude" }];
  const warnings = collectRuntimeWarnings(personas, runtimes, goose);
  assert.deepEqual(warnings, []);
});

test("collectRuntimeWarnings — persona with no runtime preference produces no warning", () => {
  const personas = [{ runtime: null }];
  const warnings = collectRuntimeWarnings(personas, runtimes, goose);
  assert.deepEqual(warnings, []);
});

test("collectRuntimeWarnings — mixed personas: matching ones are silent, non-matching emit warnings", () => {
  const personas = [{ runtime: "goose" }, { runtime: "unknown-rt" }];
  const warnings = collectRuntimeWarnings(personas, runtimes, goose);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0], /unknown-rt/);
});

test("collectRuntimeWarnings — override mode collects one warning per persona whose runtime differs from default", () => {
  const personas = [{ runtime: "goose" }, { runtime: "goose" }];
  const warnings = collectRuntimeWarnings(personas, runtimes, claude, true);
  assert.equal(warnings.length, 2);
  for (const w of warnings) {
    assert.match(w, /Runtime override/);
  }
});

test("collectRuntimeWarnings — override with one matching, one mismatching persona emits one warning", () => {
  const personas = [{ runtime: "claude" }, { runtime: "goose" }];
  const warnings = collectRuntimeWarnings(personas, runtimes, claude, true);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0], /Runtime override/);
  assert.match(warnings[0], /Goose/);
});

test("collectRuntimeWarnings — override=false behaves identically to no override flag", () => {
  const personas = [{ runtime: "goose" }, { runtime: "claude" }];
  const withoutFlag = collectRuntimeWarnings(personas, runtimes, goose);
  const withFalse = collectRuntimeWarnings(personas, runtimes, goose, false);
  assert.deepEqual(withoutFlag, withFalse);
});

test("collectRuntimeWarnings — empty personas array always returns empty", () => {
  assert.deepEqual(collectRuntimeWarnings([], runtimes, goose, true), []);
});
