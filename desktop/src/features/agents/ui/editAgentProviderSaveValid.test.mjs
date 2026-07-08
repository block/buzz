import assert from "node:assert/strict";
import test from "node:test";

import { isEditAgentProviderSaveValid } from "./AgentInstanceEditDialog.tsx";

// Helper: build args for the common "goose runtime, provider field visible" case.
const visible = true;
const hidden = false;

// ── provider field hidden ───────────────────────────────────────────────────

test("isEditAgentProviderSaveValid_fieldHidden_always_true", () => {
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: hidden,
      currentProvider: "",
      originalProvider: "",
      globalProvider: undefined,
    }),
    "provider field hidden → always valid regardless of values",
  );
});

// ── agent never had a provider (no-provider agent) ─────────────────────────

test("isEditAgentProviderSaveValid_noProviderAgent_nameEditAllowed", () => {
  // Agent was created without a provider; user edits only the name (provider
  // field stays empty). Save must remain enabled — regression from main.
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "",
      originalProvider: "",
      globalProvider: undefined,
    }),
    "no-provider agent, no global → Save must be enabled for name/timeout edits",
  );
});

test("isEditAgentProviderSaveValid_noProviderAgent_nullOriginal_allowed", () => {
  assert.ok(
    isEditAgentProviderSaveValid({
      llmProviderFieldVisible: visible,
      currentProvider: "",
      originalProvider: null,
      globalProvider: null,
    }),
    "null originalProvider treated as no provider → allowed",
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
    }),
    false,
    "whitespace-only current + global with hadProvider → still blocked",
  );
});
