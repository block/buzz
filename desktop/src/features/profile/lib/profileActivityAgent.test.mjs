import assert from "node:assert/strict";
import test from "node:test";

import { resolveProfileActivityAgent } from "./profileActivityAgent.ts";

test("uses a name-only Nostr profile for agent activity", () => {
  const agent = resolveProfileActivityAgent({
    effectivePubkey: "a".repeat(64),
    isBot: true,
    managedAgent: undefined,
    profile: {
      avatarUrl: null,
      displayName: null,
      name: "Bumble",
    },
    relayAgent: undefined,
    viewerIsOwner: true,
  });

  assert.equal(agent?.name, "Bumble");
});
