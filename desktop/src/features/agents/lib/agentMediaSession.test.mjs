import assert from "node:assert/strict";
import test from "node:test";

import {
  endRetiresSession,
  foldLiveSessions,
  isSessionExpired,
  parseAgentMediaSession,
  parseAgentMediaSessionEnd,
  trackBelongsToSessionAgent,
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

// -- trackBelongsToSessionAgent -------------------------------------------
//
// The panel plays only the announcing agent's tracks. Audio reached these
// tests the hard way: the room hook dropped every non-video track, so the
// avatar rendered silently and the surface's own comment ("hearing it is the
// point") described behaviour the code did not have.

const OTHER = "b".repeat(64);

/** A session whose announcement declares `participants`. */
function sessionWith(participants) {
  const session = parseAgentMediaSession(
    announcement({
      content: JSON.stringify({
        v: 1,
        provider: "livekit",
        connect: { url: "wss://example.livekit.cloud", room: "room-1" },
        participants,
        expires_at: EXPIRES_AT,
      }),
    }),
  );
  assert.ok(session);
  return session;
}

const SOLE_AVATAR = [{ pubkey: AGENT, tracks: ["avatar_video", "audio"] }];

test("trackBelongsToSessionAgent accepts a track whose identity names the agent", () => {
  const session = sessionWith(SOLE_AVATAR);
  for (const kind of ["video", "audio"]) {
    assert.equal(
      trackBelongsToSessionAgent(session, `anam-avatar-${AGENT}`, kind),
      true,
    );
  }
});

test("trackBelongsToSessionAgent matches an identity regardless of case", () => {
  const session = sessionWith(SOLE_AVATAR);
  assert.equal(
    trackBelongsToSessionAgent(session, AGENT.toUpperCase(), "audio"),
    true,
  );
});

test("trackBelongsToSessionAgent accepts a foreign identity when the announcement declares one publisher of that kind", () => {
  // The gateway is not obliged to put the pubkey in its provider identity, and
  // LiveKit's avatar worker joins as `anam-avatar-agent`. One declared
  // publisher makes a single track of that kind unambiguous anyway.
  const session = sessionWith(SOLE_AVATAR);
  for (const kind of ["video", "audio"]) {
    assert.equal(
      trackBelongsToSessionAgent(session, "anam-avatar-agent", kind),
      true,
    );
  }
});

test("trackBelongsToSessionAgent refuses a foreign identity once two publishers declare that kind", () => {
  const session = sessionWith([
    { pubkey: AGENT, tracks: ["avatar_video", "audio"] },
    { pubkey: OTHER, tracks: ["audio"] },
  ]);
  // Two declared audio publishers: this voice could be either, so it must not
  // play under the agent's face.
  assert.equal(
    trackBelongsToSessionAgent(session, "some-other-participant", "audio"),
    false,
  );
  // The video declaration is untouched by the second audio publisher — the
  // kinds are counted separately, so a multi-voice room still shows the face.
  assert.equal(
    trackBelongsToSessionAgent(session, "some-other-participant", "video"),
    true,
  );
});

test("trackBelongsToSessionAgent refuses a foreign identity when the announcement declares nothing", () => {
  const session = sessionWith([]);
  for (const kind of ["video", "audio"]) {
    assert.equal(
      trackBelongsToSessionAgent(session, "anam-avatar-agent", kind),
      false,
    );
  }
});

test("trackBelongsToSessionAgent counts each kind against its own declaration", () => {
  // An announcement may name a face publisher and give the voice to somebody
  // else entirely.
  const session = sessionWith([
    { pubkey: AGENT, tracks: ["avatar_video"] },
    { pubkey: OTHER, tracks: ["audio"] },
  ]);
  assert.equal(
    trackBelongsToSessionAgent(session, "unnamed", "video"),
    true,
    "one declared avatar publisher, and it is the agent",
  );
  // The one declared voice is somebody else's. That is not an ambiguity to
  // resolve in the agent's favour: the announcement says outright that this
  // session's audio is not the agent's.
  assert.equal(
    trackBelongsToSessionAgent(session, "unnamed", "audio"),
    false,
    "the sole declared audio publisher is not the agent",
  );
});

test("trackBelongsToSessionAgent refuses a viewer that joined under its own pubkey", () => {
  // The case that actually occurs, and the reason a declaration alone cannot
  // decide this. The gateway joins a viewer as `identity = <viewer pubkey>`,
  // and a v1 announcement grants viewers a microphone. Two members open one
  // session, the second unmutes, and without this check its voice replaces the
  // agent's for the first — under the agent's face.
  const session = sessionWith(SOLE_AVATAR);
  for (const kind of ["video", "audio"]) {
    assert.equal(trackBelongsToSessionAgent(session, OTHER, kind), false);
    assert.equal(
      trackBelongsToSessionAgent(session, OTHER.toUpperCase(), kind),
      false,
      "identity case must not defeat the check",
    );
    assert.equal(
      trackBelongsToSessionAgent(session, `viewer-${OTHER}`, kind),
      false,
      "a pubkey anywhere in the identity names that participant",
    );
  }
});

test("trackBelongsToSessionAgent trusts the agent's own identity over an ambiguous declaration", () => {
  // Rule order: an identity naming the agent settles it, so a room with
  // several declared voices still plays the one that proved whose it is.
  const session = sessionWith([
    { pubkey: AGENT, tracks: ["avatar_video", "audio"] },
    { pubkey: OTHER, tracks: ["audio"] },
  ]);
  assert.equal(
    trackBelongsToSessionAgent(session, `anam-avatar-${AGENT}`, "audio"),
    true,
  );
});

// -- foldLiveSessions -----------------------------------------------------
//
// Provenance: this fold used to live inside the hook and reparsed every event
// on every arrival, so each live session got a fresh object each time.
// `useAgentMediaRoom` keys a whole WebRTC connection on that object — so
// another agent going live dropped an open call, and every replayed history
// event cost its own viewer token request. These pin the identity contract that
// fixed it, which no type can express.

const START_A = "1".repeat(64);
const START_B = "2".repeat(64);
const NONE = [];

/** A 48200 that is distinct from the others by construction. */
function start({ id, pubkey = AGENT, createdAt = CREATED_AT }) {
  return announcement({
    id,
    pubkey,
    created_at: createdAt,
    content: JSON.stringify({
      v: 1,
      provider: "livekit",
      connect: { url: "wss://example.livekit.cloud", room: id },
      participants: [{ pubkey, tracks: ["avatar_video", "audio"] }],
      expires_at: createdAt + 600,
    }),
  });
}

/** A 48201 closing `startId`, signed by `signer`. */
function end({ startId, signer = AGENT }) {
  return {
    id: `d${startId.slice(1)}`,
    pubkey: signer,
    kind: 48201,
    created_at: CREATED_AT + 10,
    tags: [
      ["h", CHANNEL],
      ["e", startId],
    ],
    content: "",
  };
}

test("foldLiveSessions returns the previous array when nothing changed", () => {
  const events = [start({ id: START_A })];
  const first = foldLiveSessions(events, NONE, CREATED_AT);
  assert.equal(first.length, 1);
  assert.ok(
    Object.is(foldLiveSessions(events, first, CREATED_AT), first),
    "an unchanged fold must not re-render every consumer",
  );
});

test("foldLiveSessions keeps a live session's object when another arrives", () => {
  // The regression this exists for. An unrelated agent going live must not
  // rebuild the session being watched, or the open call is torn down and
  // rejoined for someone else's announcement.
  const a = start({ id: START_A });
  const first = foldLiveSessions([a], NONE, CREATED_AT);
  const second = foldLiveSessions(
    [a, start({ id: START_B, pubkey: OTHER, createdAt: CREATED_AT + 5 })],
    first,
    CREATED_AT,
  );
  assert.equal(second.length, 2);
  assert.ok(
    !Object.is(second, first),
    "the set changed, so the array must too",
  );
  assert.ok(
    Object.is(
      second.find((session) => session.eventId === START_A),
      first[0],
    ),
    "the watched session must still be the same object",
  );
});

test("foldLiveSessions orders newest first", () => {
  const live = foldLiveSessions(
    [
      start({ id: START_A }),
      start({ id: START_B, pubkey: OTHER, createdAt: CREATED_AT + 5 }),
    ],
    NONE,
    CREATED_AT,
  );
  assert.deepEqual(
    live.map((session) => session.eventId),
    [START_B, START_A],
  );
});

test("foldLiveSessions drops a session its owner ended", () => {
  const a = start({ id: START_A });
  const first = foldLiveSessions([a], NONE, CREATED_AT);
  const second = foldLiveSessions(
    [a, end({ startId: START_A })],
    first,
    CREATED_AT,
  );
  assert.deepEqual(second, []);
  assert.ok(!Object.is(second, first));
});

test("foldLiveSessions honours an end that arrived before its start", () => {
  // Replay order is not causal order, which is the whole reason this refolds
  // rather than updating incrementally.
  const live = foldLiveSessions(
    [end({ startId: START_A }), start({ id: START_A })],
    NONE,
    CREATED_AT,
  );
  assert.deepEqual(live, []);
});

test("foldLiveSessions keeps a session a third party tried to end", () => {
  const live = foldLiveSessions(
    [start({ id: START_A }), end({ startId: START_A, signer: OTHER })],
    NONE,
    CREATED_AT,
  );
  assert.equal(live.length, 1);
});

test("foldLiveSessions drops a session past its expiry", () => {
  const live = foldLiveSessions(
    [start({ id: START_A })],
    NONE,
    CREATED_AT + 600,
  );
  assert.deepEqual(live, []);
});

test("foldLiveSessions does not resurrect a session the events no longer carry", () => {
  // `previous` is a source of objects, never of membership.
  const first = foldLiveSessions([start({ id: START_A })], NONE, CREATED_AT);
  assert.equal(first.length, 1);
  assert.deepEqual(foldLiveSessions([], first, CREATED_AT), []);
});
