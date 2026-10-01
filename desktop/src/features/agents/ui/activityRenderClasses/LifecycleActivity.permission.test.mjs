import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { LifecycleActivity } from "./LifecycleActivity.tsx";

test("owner permission card offers only recorded ACP options and keeps native retry unavailable", () => {
  const html = renderToStaticMarkup(
    createElement(LifecycleActivity, {
      agentAvatarUrl: null,
      agentName: "Fixture agent",
      agentPubkey: "agent-pubkey",
      item: {
        id: "permission:fixture",
        type: "lifecycle",
        renderClass: "permission",
        title: "Permission requested",
        text: "Fixture action\nOptions: Allow orchid, Deny fern",
        timestamp: "2026-09-22T10:00:00Z",
        channelId: "channel-fixture",
        turnId: "turn-fixture",
        sessionId: "session-fixture",
        pendingResolution: {
          turnId: "turn-fixture",
          sessionId: "session-fixture",
          requestId: "opaque-request-9",
          actionDigest: "digest-fixture",
          options: [
            { optionId: "allow-orchid-6", label: "Allow orchid" },
            { optionId: "reject-fern-3", label: "Deny fern" },
          ],
        },
      },
    }),
  );

  assert.match(html, /Allow orchid/);
  assert.match(html, /Deny fern/);
  assert.match(html, /Auto-review retry unavailable for this adapter/);
  assert.doesNotMatch(html, /approveGuardianDeniedAction/);
});
