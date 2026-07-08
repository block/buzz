import assert from "node:assert/strict";
import test from "node:test";

import { isEditAgentProviderSaveValid } from "./AgentInstanceEditDialog.tsx";

// Shorthand for test args.
const visible = true;
const hidden = false;
const providerRuntime = true; // original runtime supported provider selection
const nonProviderRuntime = false; // original runtime did not

// ── provider field hidden ───────────────────────────────────────────────────

test("isEditAgentProviderSaveValid_fieldHidden_always_true", () => {
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: hidden,
      currentProvider: "",
      originalProvider: "",
      globalProvider: undefined,
      originalRuntimeSupportsProvider: nonProviderRuntime,
    }),
    "provider field hidden → always valid regardless of values",
  );
});

// ── legacy no-provider agent ALREADY on a provider-capable runtime ─────────

test("isEditAgentProviderSaveValid_legacyNoProvider_onProviderRuntime_nameEditAllowed", () => {
  // Agent was on goose (provider-capable) but never had a provider set.
  // User edits only the name. Save must remain enabled — regression from main.
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "",
      originalProvider: "",
      globalProvider: undefined,
      originalRuntimeSupportsProvider: providerRuntime,
    }),
    "legacy no-provider agent on provider-capable runtime → name edit allowed",
  );
});

test("isEditAgentProviderSaveValid_legacyNoProvider_nullOriginal_providerRuntime_allowed", () => {
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "",
      originalProvider: null,
      globalProvider: null,
      originalRuntimeSupportsProvider: providerRuntime,
    }),
    "null originalProvider on provider-capable runtime → treated as legacy → allowed",
  );
});

// ── fresh transition INTO a provider-capable runtime (Thufir's hole) ────────

test("isEditAgentProviderSaveValid_noProvider_switchedToProviderRuntime_blocked", () => {
  // Agent started on a non-provider runtime; user switched dropdown to goose;
  // provider stays empty. Must be BLOCKED — would create a provider-broken agent.
  assert.equal(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "",
      originalProvider: "",
      globalProvider: undefined,
      originalRuntimeSupportsProvider: nonProviderRuntime,
    }),
    false,
    "no-provider agent switched INTO provider-capable runtime → Save must be blocked",
  );
});

test("isEditAgentProviderSaveValid_noProvider_switchedToProviderRuntime_globalCovers_allowed", () => {
  // Switched to provider-capable runtime, but a global provider covers it.
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "",
      originalProvider: "",
      globalProvider: "openai",
      originalRuntimeSupportsProvider: nonProviderRuntime,
    }),
    "switched to provider runtime but global covers it → allowed",
  );
});

// ── global fallback covers an empty per-agent provider ──────────────────────

test("isEditAgentProviderSaveValid_globalFallback_coversEmpty", () => {
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "",
      originalProvider: "",
      globalProvider: "openai",
      originalRuntimeSupportsProvider: providerRuntime,
    }),
    "global fallback present → effectiveProvider resolves → allowed",
  );
});

// ── user actively clears a provider the agent had ──────────────────────────

test("isEditAgentProviderSaveValid_clearingExistingProvider_noGlobal_blocked", () => {
  // Agent originally had "openai"; user cleared the field; no global fallback.
  assert.equal(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "",
      originalProvider: "openai",
      globalProvider: undefined,
      originalRuntimeSupportsProvider: providerRuntime,
    }),
    false,
    "clearing a set provider with no global → Save must be blocked",
  );
});

test("isEditAgentProviderSaveValid_clearingExistingProvider_withGlobal_allowed", () => {
  // Agent had "openai"; user cleared it; global fallback "anthropic" covers it.
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "",
      originalProvider: "openai",
      globalProvider: "anthropic",
      originalRuntimeSupportsProvider: providerRuntime,
    }),
    "clearing per-agent provider but global covers it → allowed",
  );
});

// ── per-agent provider set directly ────────────────────────────────────────

test("isEditAgentProviderSaveValid_providerExplicitlySet_allowed", () => {
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "openai",
      originalProvider: "",
      globalProvider: undefined,
      originalRuntimeSupportsProvider: nonProviderRuntime,
    }),
    "user typed a provider → effectiveProvider resolves → allowed",
  );
});

// ── whitespace trimming ─────────────────────────────────────────────────────

test("isEditAgentProviderSaveValid_whitespaceProvider_treatedAsEmpty", () => {
  // Whitespace-only provider should NOT count as a valid provider.
  assert.equal(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "   ",
      originalProvider: "openai",
      globalProvider: "   ",
      originalRuntimeSupportsProvider: providerRuntime,
    }),
    false,
    "whitespace-only current + global with hadProvider → still blocked",
  );
});
