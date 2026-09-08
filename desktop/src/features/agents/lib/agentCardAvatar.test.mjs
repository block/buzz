import assert from "node:assert/strict";
import test from "node:test";

import {
  isAgentCardAvatarLoading,
  resolveAgentCardAvatarUrl,
} from "./agentCardAvatar.ts";

test("running agent card prefers the pubkey profile avatar", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(
      "https://relay.example/instance.png",
      "https://relay.example/definition.png",
    ),
    "https://relay.example/instance.png",
  );
});

test("running agent card falls back to the definition avatar", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(null, " https://relay.example/definition.png "),
    "https://relay.example/definition.png",
  );
});

test("running agent card ignores blank avatar values", () => {
  assert.equal(resolveAgentCardAvatarUrl("  ", ""), null);
});

test("linked agent actions wait for the authoritative profile avatar", () => {
  assert.equal(isAgentCardAvatarLoading(true, true), true);
  assert.equal(isAgentCardAvatarLoading(true, false), false);
});

test("unlinked persona actions do not wait for a profile", () => {
  assert.equal(isAgentCardAvatarLoading(false, true), false);
});
