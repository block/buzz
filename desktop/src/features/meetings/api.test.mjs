import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyMeetingError,
  decodeMeetingTokenClaims,
  normalizePlans,
  normalizeRooms,
  relayMeetingsCapability,
} from "./api.ts";

const OK_INFO = {
  meetings: {
    provider: "hivetalk",
    proxy: "/meetings",
    api_base: "https://l402relay.exe.xyz",
  },
  supported_extensions: ["nip-er", "buzz-meetings"],
};

test("relayMeetingsCapability accepts a well-formed descriptor", () => {
  assert.deepEqual(relayMeetingsCapability(OK_INFO), {
    proxyPrefix: "/meetings",
    apiBase: "https://l402relay.exe.xyz",
  });
});

test("relayMeetingsCapability strips a trailing slash from api_base", () => {
  assert.deepEqual(
    relayMeetingsCapability({
      ...OK_INFO,
      meetings: { ...OK_INFO.meetings, api_base: "https://l402relay.exe.xyz/" },
    }),
    { proxyPrefix: "/meetings", apiBase: "https://l402relay.exe.xyz" },
  );
});

test("relayMeetingsCapability rejects when the extension or provider is wrong", () => {
  assert.equal(relayMeetingsCapability({}), null);
  assert.equal(
    relayMeetingsCapability({ ...OK_INFO, supported_extensions: ["nip-er"] }),
    null,
  );
  assert.equal(
    relayMeetingsCapability({
      ...OK_INFO,
      meetings: { ...OK_INFO.meetings, provider: "jitsi" },
    }),
    null,
  );
});

test("relayMeetingsCapability rejects unsafe proxy paths", () => {
  for (const proxy of [
    "https://attacker.example/meetings",
    "//attacker.example",
    "/\\attacker",
    "/%2e%2e/admin",
    "/meetings/../admin",
    "/meetings?x=1",
    "/meetings#f",
  ]) {
    assert.equal(
      relayMeetingsCapability({
        ...OK_INFO,
        meetings: { ...OK_INFO.meetings, proxy },
      }),
      null,
      `proxy ${proxy} must be rejected`,
    );
  }
});

test("relayMeetingsCapability requires an https api_base", () => {
  for (const api_base of [
    "http://l402relay.exe.xyz",
    "ftp://l402relay.exe.xyz",
    "l402relay.exe.xyz",
    "",
    "not a url",
  ]) {
    assert.equal(
      relayMeetingsCapability({
        ...OK_INFO,
        meetings: { ...OK_INFO.meetings, api_base },
      }),
      null,
      `api_base ${api_base} must be rejected`,
    );
  }
});

test("classifyMeetingError maps every documented status/reason", () => {
  const cases = [
    [401, {}, "bad_signature"],
    [402, { reason: "subscription_required" }, "subscription_required"],
    [402, { reason: "subscription_expired" }, "subscription_expired"],
    [403, { reason: "subscription_expired" }, "subscription_expired"],
    [403, { reason: "room_not_registered" }, "room_not_registered"],
    [
      403,
      { reason: "ephemeral_rooms_are_dashboard_only" },
      "ephemeral_rooms_are_dashboard_only",
    ],
    [403, { error: "relay_membership_required" }, "membership_required"],
    [404, { error: "meetings is not configured" }, "not_configured"],
    // Forwarded from HiveTalk now that the relay preserves the upstream status
    // on a plain-text error body instead of rewriting it to a 502.
    [404, { error: "Room not found" }, "room_not_registered"],
    [409, { reason: "pending_invoice" }, "pending_invoice"],
    [429, { error: "rate-limited: retry in 12s" }, "rate_limited"],
    [502, { error: "meeting provider is unavailable" }, "provider_unavailable"],
    [503, {}, "provider_unavailable"],
    [418, { error: "teapot" }, "unknown"],
  ];
  for (const [status, body, kind] of cases) {
    const err = classifyMeetingError(status, body);
    assert.equal(err.kind, kind, `${status} ${JSON.stringify(body)}`);
    assert.equal(err.status, status);
  }
});

test("classifyMeetingError parses retry-after seconds from a 429", () => {
  const err = classifyMeetingError(429, {
    error: "rate-limited: meeting action quota exceeded; retry in 7s",
  });
  assert.equal(err.retryAfterSecs, 7);
});

test("classifyMeetingError falls through to the raw error on an unknown status", () => {
  const err = classifyMeetingError(500, { error: "boom" });
  assert.equal(err.kind, "unknown");
  assert.equal(err.message, "boom");
});

test("decodeMeetingTokenClaims reads owner/moderator/room from a HiveTalk JWT", () => {
  const payload = {
    owner: true,
    moderator: true,
    video: { room: "room-abc" },
  };
  const jwt = `h.${Buffer.from(JSON.stringify(payload)).toString("base64url")}.s`;
  assert.deepEqual(decodeMeetingTokenClaims(jwt), {
    owner: true,
    moderator: true,
    room: "room-abc",
  });
});

test("decodeMeetingTokenClaims treats a plain participant token as non-host", () => {
  const jwt = `h.${Buffer.from(
    JSON.stringify({ video: { room: "r", canPublish: true } }),
  ).toString("base64url")}.s`;
  assert.deepEqual(decodeMeetingTokenClaims(jwt), {
    owner: false,
    moderator: false,
    room: "r",
  });
});

test("decodeMeetingTokenClaims decodes a payload whose length isn't a multiple of 4", () => {
  // Hand-build a base64url payload that needs `=` padding restored before atob.
  const b64 = Buffer.from(JSON.stringify({ owner: true })).toString("base64");
  const b64url = b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  assert.notEqual(b64url.length % 4, 0, "fixture must be un-padded");
  const jwt = `h.${b64url}.s`;
  assert.deepEqual(decodeMeetingTokenClaims(jwt), {
    owner: true,
    moderator: true,
    room: null,
  });
});

test("decodeMeetingTokenClaims never throws on garbage", () => {
  for (const bad of ["", "not-a-jwt", "a.b", "a..c", "x.@@@.y"]) {
    assert.deepEqual(decodeMeetingTokenClaims(bad), {
      owner: false,
      moderator: false,
      room: null,
    });
  }
});

// Live HiveTalk body, captured from `GET https://l402relay.exe.xyz/api/plans`.
const HIVETALK_PLANS = {
  free_quota: 3,
  plans: [
    { id: "standard_1y", room_quota: 3, days: 365, price_sats: 360 },
    { id: "bulk10_2y", room_quota: 10, days: 730, price_sats: 850 },
  ],
};

test("normalizePlans unwraps the HiveTalk envelope and maps field names", () => {
  assert.deepEqual(normalizePlans(HIVETALK_PLANS), [
    {
      id: "standard_1y",
      plan: "standard_1y",
      room_quota: 3,
      days: 365,
      price_sats: 360,
      amount_sats: 360,
      period: "year",
    },
    {
      id: "bulk10_2y",
      plan: "bulk10_2y",
      room_quota: 10,
      days: 730,
      price_sats: 850,
      amount_sats: 850,
      period: "2 years",
    },
  ]);
});

test("normalizePlans accepts a bare array and native field names", () => {
  assert.deepEqual(
    normalizePlans([{ plan: "standard_1y", amount_sats: 360, period: "year" }]),
    [{ plan: "standard_1y", amount_sats: 360, period: "year" }],
  );
});

test("normalizePlans drops entries that cannot render a card", () => {
  assert.deepEqual(
    normalizePlans({
      plans: [
        { id: "no_price" },
        { price_sats: 100 },
        null,
        "nope",
        { id: "ok", price_sats: 10 },
      ],
    }),
    [{ id: "ok", plan: "ok", price_sats: 10, amount_sats: 10 }],
  );
});

test("normalizePlans returns [] for shapes it does not understand", () => {
  for (const bad of [null, undefined, 7, "plans", {}, { plans: {} }]) {
    assert.deepEqual(normalizePlans(bad), []);
  }
});

// Live `GET /meetings/rooms-by-pubkey` entry — registered-room shape.
const REGISTERED_ROOM = {
  audience_mode: false,
  created_via: "api",
  lobby_enabled: false,
  locked: false,
  mute_on_join: false,
  pubkey: "6ed800fd1545fbe4340a4bfd207cb1c11f84dce55bb5d9dfeaefb603e4390009",
  room_id: "c1015117-b518-4e3b-b948-ae9ddfc362f4",
  room_name: "buzz-meet-spike-s02",
  status: "open",
  updated_at: "2026-08-27T03:29:25.943019Z",
  username: "spike-guest",
};

test("normalizeRooms maps rooms-by-pubkey room_name onto name", () => {
  const [room] = normalizeRooms([REGISTERED_ROOM]);
  assert.equal(room.name, "buzz-meet-spike-s02");
  assert.equal(room.locked, false);
  assert.equal(room.numParticipants, undefined);
});

test("normalizeRooms passes a LiveKit list-rooms entry through", () => {
  assert.deepEqual(normalizeRooms([{ name: "standup", numParticipants: 3 }]), [
    { name: "standup", numParticipants: 3 },
  ]);
});

test("normalizeRooms accepts snake_case num_participants", () => {
  const [room] = normalizeRooms([{ name: "standup", num_participants: 2 }]);
  assert.equal(room.numParticipants, 2);
});

test("normalizeRooms drops entries with no usable name", () => {
  assert.deepEqual(
    normalizeRooms([{ room_id: "x" }, { name: "  " }, null, "nope", 7]),
    [],
  );
});

test("normalizeRooms returns [] for a non-array body", () => {
  for (const bad of [null, undefined, {}, { rooms: [] }, "[]"]) {
    assert.deepEqual(normalizeRooms(bad), []);
  }
});
