import assert from "node:assert/strict";
import { describe, it } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { AgentTurnFailureStatus } from "./AgentTurnFailureStatus.tsx";

const AGENT = "a".repeat(64);

function renderFailure(disposition = "retrying", overrides = {}) {
  return renderToStaticMarkup(
    React.createElement(AgentTurnFailureStatus, {
      agents: [{ pubkey: AGENT, name: "Review Bee" }],
      failure: {
        agentPubkey: AGENT,
        turnId: "turn-1",
        channelId: "channel-1",
        rootEventId: "1".repeat(64),
        parentEventId: "2".repeat(64),
        triggeringEventIds: ["2".repeat(64)],
        error: "Upgrade the adapter and try again.",
        disposition,
        attempt: 2,
        timestamp: "2026-09-09T17:49:09Z",
        ...overrides,
      },
      onOpenAgentSession() {},
      profiles: {},
    }),
  );
}

describe("AgentTurnFailureStatus", () => {
  it("renders an actionable retry failure in the composer activity rail", () => {
    const html = renderFailure();

    assert.match(html, /data-testid="agent-turn-failure-status"/);
    assert.match(html, /Review Bee couldn/);
    assert.match(html, /Retrying automatically/);
    assert.match(html, /attempt 2/);
    assert.match(html, /View activity/);
    assert.match(html, /title="Upgrade the adapter and try again\."/);
  });

  it("renders terminal disposition copy", () => {
    assert.match(
      renderFailure("dead_lettered"),
      /Stopped after multiple attempts/,
    );
  });
});

it("renders legacy correlation and recovery as unknown", () => {
  const html = renderFailure("unknown", {
    rootEventId: null,
    parentEventId: null,
  });
  assert.match(html, /Conversation unknown/);
  assert.match(html, /Recovery status unknown/);
  assert.doesNotMatch(html, /Stopped/);
});
