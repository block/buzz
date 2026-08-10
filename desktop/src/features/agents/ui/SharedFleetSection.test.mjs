import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  currentSharedFleetRows,
  currentSharedTeamCatalog,
  SharedFleetContent,
} from "./SharedFleetSection.tsx";

const PUBKEY = "a".repeat(64);

function render(overrides = {}) {
  return renderToStaticMarkup(
    React.createElement(SharedFleetContent, {
      fleetError: null,
      isFleetLoading: false,
      isTeamsLoading: false,
      rows: [
        {
          pubkey: PUBKEY,
          name: "Clyde",
          modelLabel: "Model Alpha",
          status: "online",
          channels: ["Agent Testing", "x-articles-ozark"],
          mentionHint: "Mention only in its 2 assigned channels",
        },
      ],
      teamError: null,
      teams: [
        {
          eventId: "c".repeat(64),
          ownerPubkey: "b".repeat(64),
          teamDTag: "deepseek-crew",
          name: "DeepSeek Crew",
          memberCount: 5,
          memberKeys: [],
        },
      ],
      ...overrides,
    }),
  );
}

test("renders live remote worker and safe shared team fields", () => {
  const html = render();
  assert.match(html, /Shared fleet/);
  assert.match(html, /Remote worker/);
  assert.match(html, /Clyde/);
  assert.match(html, /Model Alpha/);
  assert.match(html, /Agent Testing/);
  assert.match(html, /DeepSeek Crew/);
  assert.match(html, /5 members/);
  assert.doesNotMatch(html, />#x-articles</);
  assert.doesNotMatch(html, /c{64}|memberKeys|provider|filesystem/);
});

test("renders no lifecycle, deployment, or global mention controls", () => {
  const html = render();
  assert.doesNotMatch(
    html,
    /\b(Start|Stop|Restart|Deploy|Delete|Edit|Create|Add to channel)\b/,
  );
  assert.doesNotMatch(html, /Mention all|Mention everywhere/);
  assert.doesNotMatch(html, /system_prompt|provider|filesystem|credential/);
});

test("failed current reads do not retain stale fleet or catalog rows", () => {
  const staleRows = currentSharedFleetRows(
    [
      {
        pubkey: PUBKEY,
        name: "Clyde",
        model: null,
        agentType: "agent",
        channels: [],
        channelIds: [],
        capabilities: [],
        status: "online",
        respondTo: null,
        respondToAllowlist: [],
      },
    ],
    { [PUBKEY]: "online" },
    { relayAgentsAreCurrent: false, presenceIsCurrent: true },
  );
  const staleTeams = currentSharedTeamCatalog(
    [
      {
        eventId: "d".repeat(64),
        ownerPubkey: "b".repeat(64),
        teamDTag: "stale",
        name: "Stale Team",
        memberCount: 1,
        memberKeys: [],
      },
    ],
    false,
  );
  assert.deepEqual(staleRows, []);
  assert.deepEqual(staleTeams, []);
});
