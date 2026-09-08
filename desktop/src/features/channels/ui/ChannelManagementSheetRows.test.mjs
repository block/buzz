import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ChannelHero } from "./ChannelManagementSheetRows.tsx";

function channel(overrides = {}) {
  return {
    archivedAt: null,
    channelType: "stream",
    description: "First paragraph.\n\nSecond paragraph.\nThird line.",
    id: "11111111-1111-4111-8111-111111111111",
    isMember: true,
    lastMessageAt: null,
    memberCount: 1,
    memberPubkeys: [],
    name: "test",
    participantPubkeys: [],
    participants: [],
    purpose: null,
    topic: null,
    ttlDeadline: null,
    ttlSeconds: null,
    visibility: "open",
    ...overrides,
  };
}

function renderHero(props) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { enabled: false } },
  });
  return renderToStaticMarkup(
    React.createElement(
      QueryClientProvider,
      { client: queryClient },
      React.createElement(ChannelHero, props),
    ),
  );
}

function assertMultilineDescriptionClasses(html) {
  const descriptionTag = html.match(
    /<(?:span|p)[^>]*data-testid="channel-management-description"[^>]*>/,
  )?.[0];
  assert.ok(descriptionTag, "channel-management description must render");
  assert.match(descriptionTag, /whitespace-pre-line/);
  assert.match(descriptionTag, /line-clamp-6/);
  assert.doesNotMatch(descriptionTag, /line-clamp-2/);
}

test("editable ChannelHero preserves paragraph layout within a six-line clamp", () => {
  const html = renderHero({
    channel: channel(),
    onEdit() {},
  });

  assert.match(html, /data-testid="channel-management-edit"/);
  assertMultilineDescriptionClasses(html);
});

test("read-only ChannelHero preserves paragraph layout within a six-line clamp", () => {
  const html = renderHero({ channel: channel() });

  assert.doesNotMatch(html, /data-testid="channel-management-edit"/);
  assertMultilineDescriptionClasses(html);
});
