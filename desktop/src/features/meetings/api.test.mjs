import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyMeetingError,
  decodeMeetingTokenClaims,
  relayMeetingsCapability,
} from "./api.ts";

const OK_INFO = {
  meetings: {
    provider: "hivetalk",
    proxy: "/meetings",
    api_base: "https://premrelay.exe.xyz",
  },
  supported_extensions: ["nip-er", "buzz-meetings"],
};

test("relayMeetingsCapability accepts a well-formed descriptor", () => {
  assert.deepEqual(relayMeetingsCapability(OK_INFO), {
    proxyPrefix: "/meetings",
    apiBase: "https://premrelay.exe.xyz",
  });
});

test("relayMeetingsCapability strips a trailing slash from api_base", () => {
  assert.deepEqual(
    relayMeetingsCapability({
      ...OK_INFO,
      meetings: { ...OK_INFO.meetings, api_base: "https://premrelay.exe.xyz/" },
    }),
    { proxyPrefix: "/meetings", apiBase: "https://premrelay.exe.xyz" },
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
    "http://premrelay.exe.xyz",
    "ftp://premrelay.exe.xyz",
    "premrelay.exe.xyz",
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
