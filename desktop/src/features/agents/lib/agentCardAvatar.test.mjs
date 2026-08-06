import assert from "node:assert/strict";
import test from "node:test";

import { resolveAgentCardAvatarUrl } from "./agentCardAvatar.ts";

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
