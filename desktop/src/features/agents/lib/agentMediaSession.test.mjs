import assert from "node:assert/strict";
import test from "node:test";

import {
  endRetiresSession,
  isSessionExpired,
  parseAgentMediaSession,
  parseAgentMediaSessionEnd,
} from "./agentMediaSession.ts";

const AGENT = "a".repeat(64);
const CHANNEL = "3f1c9f5a-1111-4222-8333-444455556666";
const CREATED_AT = 1_700_000_000;
const EXPIRES_AT = CREATED_AT + 600;

/** A well-formed body, so each test states only what it is about. */
function body(overrides = {}) {
  return JSON.stringify({
    provider: "livekit",
    connect: { url: "wss://x", room: "r" },
    expires_at: EXPIRES_AT,
    ...overrides,
  });
}

function announcement(overrides = {}) {
  const { content, tags, ...rest } = overrides;
  return {
    id: "e".repeat(64),
    pubkey: AGENT,
    kind: 48200,
    created_at: CREATED_AT,
    tags: tags ?? [["h", CHANNEL]],
    content:
      content ??
      JSON.stringify({
        v: 1,
        provider: "livekit",
        connect: { url: "wss://example.livekit.cloud", room: "room-1" },
        token_endpoint: "https://gateway.example/media/token",
        participants: [{ pubkey: AGENT, tracks: ["avatar_video", "audio"] }],
        viewer: { subscribe: ["avatar_video", "audio"], publish: ["audio"] },
        expires_at: EXPIRES_AT,
      }),
    ...rest,
  };
}

test("parseAgentMediaSession reads a v1 announcement", () => {
  const session = parseAgentMediaSession(announcement());
  assert.ok(session);
  assert.equal(session.agentPubkey, AGENT);
  assert.equal(session.channelId, CHANNEL);
  assert.equal(session.serverUrl, "wss://example.livekit.cloud");
  assert.equal(session.room, "room-1");
  assert.equal(session.tokenEndpoint, "https://gateway.example/media/token");
  assert.equal(session.startedAt, CREATED_AT);
  assert.equal(session.expiresAt, EXPIRES_AT);
});

test("parseAgentMediaSession takes ownership from the signature, not a p tag", () => {
  const impostor = "b".repeat(64);
  const session = parseAgentMediaSession(
    announcement({
      tags: [
        ["h", CHANNEL],
        ["p", impostor],
      ],
    }),
  );
  assert.equal(session?.agentPubkey, AGENT);
});

test("parseAgentMediaSession keeps the triggering message id", () => {
  const source = "c".repeat(64);
  const session = parseAgentMediaSession(
    announcement({
      tags: [
        ["h", CHANNEL],
        ["e", source],
      ],
    }),
  );
  assert.equal(session?.sourceEventId, source);
});

test("parseAgentMediaSession returns null without an h tag", () => {
  assert.equal(parseAgentMediaSession(announcement({ tags: [] })), null);
});

test("parseAgentMediaSession returns null on non-JSON content", () => {
  assert.equal(parseAgentMediaSession(announcement({ content: "nope" })), null);
});

test("parseAgentMediaSession returns null for a provider it cannot connect to", () => {
  const content = body({ provider: "hologram-net" });
  assert.equal(parseAgentMediaSession(announcement({ content })), null);
});

test("parseAgentMediaSession returns null without usable connect params", () => {
  for (const connect of [
    {},
    { url: "wss://x" },
    { room: "r" },
    { url: "", room: "r" },
  ]) {
    const content = JSON.stringify({
      provider: "livekit",
      connect,
      expires_at: EXPIRES_AT,
    });
    assert.equal(
      parseAgentMediaSession(announcement({ content })),
      null,
      `expected ${JSON.stringify(connect)} to be rejected`,
    );
  }
});

test("parseAgentMediaSession drops a non-http token endpoint but keeps the session", () => {
  // A session with no endpoint is one this viewer cannot get a token for —
  // still worth surfacing, unlike a javascript:/file: URL we would ever fetch.
  const content = body({ token_endpoint: "javascript:alert(1)" });
  const session = parseAgentMediaSession(announcement({ content }));
  assert.ok(session);
  assert.equal(session.tokenEndpoint, null);
});

test("parseAgentMediaSession ignores track kinds it does not know", () => {
  const content = body({
    viewer: { subscribe: ["avatar_video", "hologram"], publish: ["audio", 7] },
  });
  const session = parseAgentMediaSession(announcement({ content }));
  assert.deepEqual(session?.viewer.subscribe, ["avatar_video"]);
  assert.deepEqual(session?.viewer.publish, ["audio"]);
});

test("parseAgentMediaSession skips malformed participant entries", () => {
  const content = body({
    participants: [{ tracks: ["audio"] }, null, { pubkey: AGENT, tracks: [] }],
  });
  const session = parseAgentMediaSession(announcement({ content }));
  assert.equal(session?.participants.length, 1);
  assert.equal(session?.participants[0].pubkey, AGENT);
});

test("parseAgentMediaSession fails closed without a usable expires_at", () => {
  // No default: any default is a guess about how long a room this client cannot
  // see stays alive, and the generous guess keeps a dead card on screen.
  for (const expires of [undefined, null, "later", 1.5, Number.NaN]) {
    const record = {
      provider: "livekit",
      connect: { url: "wss://x", room: "r" },
    };
    if (expires !== undefined) record.expires_at = expires;
    assert.equal(
      parseAgentMediaSession(announcement({ content: JSON.stringify(record) })),
      null,
      `expected expires_at ${String(expires)} to be rejected`,
    );
  }
});

test("parseAgentMediaSession rejects an expiry at or before the announcement", () => {
  for (const expires of [CREATED_AT, CREATED_AT - 1]) {
    const content = body({ expires_at: expires });
    assert.equal(
      parseAgentMediaSession(announcement({ content })),
      null,
      `expected expires_at ${expires} to be rejected`,
    );
  }
});

test("isSessionExpired turns over at the announced second", () => {
  const session = parseAgentMediaSession(announcement());
  assert.ok(session);
  assert.equal(isSessionExpired(session, EXPIRES_AT - 1), false);
  assert.equal(isSessionExpired(session, EXPIRES_AT), true);
  assert.equal(isSessionExpired(session, EXPIRES_AT + 1), true);
});

test("parseAgentMediaSessionEnd names the start it closes", () => {
  const start = "d".repeat(64);
  const end = parseAgentMediaSessionEnd({
    pubkey: AGENT,
    tags: [
      ["h", CHANNEL],
      ["e", start],
    ],
  });
  assert.equal(end?.startEventId, start);
  assert.equal(end?.signer, AGENT);
  assert.equal(end?.subject, null);
});

test("parseAgentMediaSessionEnd returns null when it names no start", () => {
  assert.equal(
    parseAgentMediaSessionEnd({ pubkey: AGENT, tags: [["h", CHANNEL]] }),
    null,
  );
});

test("parseAgentMediaSessionEnd reads a lone p tag as the subject", () => {
  const relay = "f".repeat(64);
  const start = "d".repeat(64);
  const end = parseAgentMediaSessionEnd({
    pubkey: relay,
    tags: [
      ["e", start],
      ["p", AGENT],
    ],
  });
  assert.equal(end?.subject, AGENT);

  // Several p tags name nobody in particular, so there is no subject.
  const ambiguous = parseAgentMediaSessionEnd({
    pubkey: relay,
    tags: [
      ["e", start],
      ["p", AGENT],
      ["p", "b".repeat(64)],
    ],
  });
  assert.equal(ambiguous?.subject, null);
});

test("endRetiresSession honours the session owner", () => {
  const session = parseAgentMediaSession(announcement());
  assert.ok(session);
  const end = parseAgentMediaSessionEnd({
    pubkey: AGENT,
    tags: [["e", session.eventId]],
  });
  assert.ok(end);
  assert.equal(endRetiresSession(end, session), true);
});

test("endRetiresSession honours a relay-signed end naming the owner", () => {
  const session = parseAgentMediaSession(announcement());
  assert.ok(session);
  const end = parseAgentMediaSessionEnd({
    pubkey: "f".repeat(64),
    tags: [
      ["e", session.eventId],
      ["p", AGENT],
    ],
  });
  assert.ok(end);
  assert.equal(endRetiresSession(end, session), true);
});

test("endRetiresSession refuses a third party", () => {
  // Otherwise any member could retire another agent's live card by publishing
  // a 48201 that names its start.
  const session = parseAgentMediaSession(announcement());
  assert.ok(session);
  const end = parseAgentMediaSessionEnd({
    pubkey: "b".repeat(64),
    tags: [["e", session.eventId]],
  });
  assert.ok(end);
  assert.equal(endRetiresSession(end, session), false);
});

test("endRetiresSession refuses an end naming a different start", () => {
  const session = parseAgentMediaSession(announcement());
  assert.ok(session);
  const end = parseAgentMediaSessionEnd({
    pubkey: AGENT,
    tags: [["e", "9".repeat(64)]],
  });
  assert.ok(end);
  assert.equal(endRetiresSession(end, session), false);
});
