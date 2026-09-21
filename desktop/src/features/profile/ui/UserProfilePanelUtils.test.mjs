import assert from "node:assert/strict";
import test from "node:test";

import {
  parseProfilePanelTab,
  parseProfilePanelView,
  profileManagedAgentCreateInput,
  profilePanelTabFromSearch,
  profilePanelTargetKey,
  profilePanelViewFromSearch,
} from "./UserProfilePanelUtils.ts";

test("profileManagedAgentCreateInput makes ambiguous retries idempotent", () => {
  assert.deepEqual(
    profileManagedAgentCreateInput({ name: "Fizz", personaId: "persona-1" }),
    {
      name: "Fizz",
      personaId: "persona-1",
      reuseExistingPersonaInstance: true,
    },
  );
});

test("parseProfilePanelView accepts all profile panel subviews", () => {
  for (const view of [
    "summary",
    "info",
    "configuration",
    "diagnostics",
    "memories",
    "channels",
    "logs",
  ]) {
    assert.equal(parseProfilePanelView(view), view);
  }
});

test("parseProfilePanelView maps legacy agent config subviews to configuration", () => {
  for (const view of ["model", "settings"]) {
    assert.equal(parseProfilePanelView(view), "configuration");
  }
});

test("profilePanelViewFromSearch falls back to summary for invalid values", () => {
  assert.equal(parseProfilePanelView("missing"), null);
  assert.equal(profilePanelViewFromSearch("missing"), "summary");
  assert.equal(profilePanelViewFromSearch(null), "summary");
});

test("parseProfilePanelTab accepts profile summary tabs", () => {
  for (const tab of ["info", "runtime", "channels", "memories"]) {
    assert.equal(parseProfilePanelTab(tab), tab);
  }
});

test("profilePanelTabFromSearch falls back to info for invalid values", () => {
  assert.equal(parseProfilePanelTab("missing"), null);
  assert.equal(profilePanelTabFromSearch("missing"), "info");
  assert.equal(profilePanelTabFromSearch(null), "info");
});

test("profile target identity stays stable while a requested pubkey is canonicalized", () => {
  const historicalPubkey = "a".repeat(64);
  assert.equal(
    profilePanelTargetKey(historicalPubkey, undefined),
    historicalPubkey,
  );
  assert.equal(
    profilePanelTargetKey(historicalPubkey, "resolved-persona"),
    historicalPubkey,
  );
  assert.equal(
    profilePanelTargetKey(undefined, "requested-persona"),
    "persona:requested-persona",
  );
});
