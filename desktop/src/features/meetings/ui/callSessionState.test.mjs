import assert from "node:assert/strict";
import test from "node:test";

import {
  CALL_SESSION_CAP_MS,
  CALL_SESSION_SOFT_WARN_MS,
  connectionBannerModel,
  disconnectBannerModel,
  formatCallElapsed,
  sessionCapBannerModel,
} from "./callSessionState.ts";

test("connectionBannerModel: connected is unobstructed", () => {
  assert.equal(connectionBannerModel("connected"), null);
});

test("connectionBannerModel: connecting + reconnecting never offer rejoin", () => {
  assert.equal(connectionBannerModel("connecting").showRejoin, false);
  const reconnecting = connectionBannerModel("reconnecting");
  assert.equal(reconnecting.tone, "warning");
  assert.equal(reconnecting.showRejoin, false);
});

test("connectionBannerModel: a bare disconnect is a retryable error", () => {
  const model = connectionBannerModel("disconnected");
  assert.equal(model.tone, "error");
  assert.equal(model.showRejoin, true);
});

test("disconnectBannerModel: a clean user leave shows nothing", () => {
  assert.equal(disconnectBannerModel("user"), null);
});

test("disconnectBannerModel: server/duplicate/session-ended all offer rejoin", () => {
  for (const reason of ["server", "duplicate", "session-ended", "unknown"]) {
    assert.equal(disconnectBannerModel(reason).showRejoin, true);
  }
  assert.equal(disconnectBannerModel("session-ended").tone, "info");
  assert.equal(disconnectBannerModel("server").tone, "error");
});

test("sessionCapBannerModel: quiet until the soft threshold", () => {
  assert.equal(sessionCapBannerModel(0), null);
  assert.equal(sessionCapBannerModel(CALL_SESSION_SOFT_WARN_MS - 1), null);
  assert.equal(sessionCapBannerModel(Number.NaN), null);
});

test("sessionCapBannerModel: counts down between the soft warn and the cap", () => {
  const model = sessionCapBannerModel(CALL_SESSION_SOFT_WARN_MS);
  assert.equal(model.tone, "warning");
  assert.match(model.title, /ends in about 30 min/);
  assert.equal(sessionCapBannerModel(CALL_SESSION_CAP_MS).showRejoin, false);
  assert.match(
    sessionCapBannerModel(CALL_SESSION_CAP_MS).title,
    /about to end/,
  );
});

test("formatCallElapsed: m:ss under an hour, h:mm:ss past it", () => {
  assert.equal(formatCallElapsed(0), "0:00");
  assert.equal(formatCallElapsed(65_000), "1:05");
  assert.equal(formatCallElapsed(3_661_000), "1:01:01");
  assert.equal(formatCallElapsed(-5), "0:00");
});
