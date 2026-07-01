import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { RawRailActivity } from "./RawRailActivity.tsx";

const baseProps = {
  agentAvatarUrl: null,
  agentName: "Test Agent",
  agentPubkey: "pubkey123",
};
const baseTimestamp = "2026-06-14T19:00:00.000Z";
const baseIdentity = {
  agentPubkey: "pubkey123",
  sessionId: "session-001",
  turnId: null,
};

test("RawRailActivity render: raw_json_rpc keeps <pre> safety net", () => {
  const html = renderToStaticMarkup(
    React.createElement(RawRailActivity, {
      ...baseProps,
      item: {
        ...baseIdentity,
        id: "meta:raw",
        type: "metadata",
        renderClass: "raw-rail",
        title: "Raw ACP payload",
        sections: [{ title: "body", body: "{}" }],
        timestamp: baseTimestamp,
        acpSource: "raw_json_rpc",
      },
    }),
  );
  assert.ok(html.includes("<pre"), "raw_json_rpc should render <pre>");
  assert.ok(
    !html.includes("rounded-2xl"),
    "raw_json_rpc should not render polished accordion",
  );
});

test("RawRailActivity render: system prompt (no acpSource) uses polished accordion", () => {
  const html = renderToStaticMarkup(
    React.createElement(RawRailActivity, {
      ...baseProps,
      item: {
        ...baseIdentity,
        id: "meta:sys",
        type: "metadata",
        renderClass: "raw-rail",
        title: "System prompt",
        sections: [{ title: "Instructions", body: "You are an agent." }],
        timestamp: baseTimestamp,
      },
    }),
  );
  assert.ok(
    html.includes("rounded-2xl"),
    "system prompt should render polished accordion",
  );
  assert.ok(!html.includes("<pre"), "system prompt should not render <pre>");
});

test("RawRailActivity render: steer-turn prompt context uses polished accordion", () => {
  const html = renderToStaticMarkup(
    React.createElement(RawRailActivity, {
      ...baseProps,
      item: {
        ...baseIdentity,
        id: "meta:ctx",
        type: "metadata",
        renderClass: "raw-rail",
        title: "Prompt context",
        sections: [{ title: "Thread history", body: "..." }],
        timestamp: baseTimestamp,
        acpSource: "session/prompt:context",
      },
    }),
  );
  assert.ok(
    html.includes("rounded-2xl"),
    "steer-turn prompt context should render polished accordion",
  );
  assert.ok(
    !html.includes("<pre"),
    "steer-turn prompt context should not render <pre>",
  );
});
