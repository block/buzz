import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { LivingShipCanvas } from "./LivingShipCanvas.tsx";

const agents = [
  {
    adviser: "operations",
    personaId: "builtin:command-operations",
    pubkey: "ops-key",
    name: "Operations",
    label: "Operations",
    shortLabel: "OPS",
    spriteColumn: 1,
    lifecycle: "online",
    working: true,
    locationId: "cic",
    locationReason: "working-home",
    channelId: "channel-1",
    channelName: "operations",
    workingSince: 100,
    taskSummary: "Maintaining the operational picture",
    collaborationId: null,
    collaboratorPubkeys: [],
  },
  {
    adviser: "intelligence",
    personaId: "builtin:command-intelligence",
    pubkey: "n2-key",
    name: "Maritime N2",
    label: "Maritime N2",
    shortLabel: "N2",
    spriteColumn: 2,
    lifecycle: "offline",
    working: false,
    locationId: "personnel-strip",
    locationReason: "unavailable",
    channelId: null,
    channelName: null,
    workingSince: null,
    taskSummary: null,
    collaborationId: null,
    collaboratorPubkeys: [],
  },
];

test("renders the full ship workspace as semantic rooms and agent controls", () => {
  const html = renderToStaticMarkup(
    React.createElement(LivingShipCanvas, {
      agents,
      onSelectAgent() {},
      onSelectRoom() {},
      selectedAgentPubkey: null,
      selectedRoomId: null,
    }),
  );

  for (const room of [
    "DSE Operator Room",
    "Plans Room",
    "C.I.C.",
    "Chart House",
    "Wardroom",
    "Meeting Room",
    "Ship&#x27;s Office",
    "Supply Office",
  ]) {
    assert.match(html, new RegExp(room));
  }
  assert.match(html, /Personnel not aboard/);
  assert.match(html, /aria-label="Select Operations in C\.I\.C\."/);
  assert.match(html, /aria-label="Select Maritime N2, not aboard"/);
  assert.match(html, /data-state="working"/);
  assert.match(html, /data-state="offline"/);
});

test("projects room and agent controls into the native ship artwork coordinates", () => {
  const html = renderToStaticMarkup(
    React.createElement(LivingShipCanvas, {
      agents,
      onSelectAgent() {},
      onSelectRoom() {},
      selectedAgentPubkey: null,
      selectedRoomId: null,
    }),
  );

  assert.match(
    html,
    /data-room-id="cic"[^>]*style="--room-x:66\.704675%;--room-y:49\.888393%;--room-width:7\.981756%;--room-height:9\.151786%"/,
  );
  assert.match(
    html,
    /aria-label="Select Operations in C\.I\.C\."[^>]*style="--agent-x:67\.388826%;--agent-y:50\.334821%;/,
  );
});
