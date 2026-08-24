import assert from "node:assert/strict";
import test from "node:test";

import { shouldShowAgentSessionUnavailable } from "./ChannelPane.helpers.ts";

test("unavailable activity state is exclusive to dedicated windows", () => {
  const unresolvedSession = {
    openAgentSessionPubkey: "agent-pubkey",
    selectedAgent: null,
  };

  assert.equal(
    shouldShowAgentSessionUnavailable({
      ...unresolvedSession,
      isDedicatedActivityWindow: true,
    }),
    true,
  );
  assert.equal(
    shouldShowAgentSessionUnavailable({
      ...unresolvedSession,
      isDedicatedActivityWindow: false,
    }),
    false,
  );
});
