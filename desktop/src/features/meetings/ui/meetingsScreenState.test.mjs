import assert from "node:assert/strict";
import test from "node:test";

import { MeetingError } from "../api.ts";
import {
  isHostingSetupError,
  isRoomLive,
  selectMeetingsView,
} from "./meetingsScreenState.ts";

test("selectMeetingsView: no capability + loading -> loading", () => {
  assert.deepEqual(
    selectMeetingsView({
      hasCapability: false,
      isCapabilityLoading: true,
      deepLink: {},
    }),
    { kind: "loading" },
  );
});

test("selectMeetingsView: no capability + settled -> unavailable", () => {
  assert.deepEqual(
    selectMeetingsView({
      hasCapability: false,
      isCapabilityLoading: false,
      deepLink: {},
    }),
    { kind: "unavailable" },
  );
});

test("selectMeetingsView: join deep link -> call view", () => {
  assert.deepEqual(
    selectMeetingsView({
      hasCapability: true,
      isCapabilityLoading: false,
      deepLink: { action: "join", room: "weekly-sync" },
    }),
    { kind: "call", room: "weekly-sync" },
  );
});

test("selectMeetingsView: start deep link -> list with prefill + focus", () => {
  assert.deepEqual(
    selectMeetingsView({
      hasCapability: true,
      isCapabilityLoading: false,
      deepLink: { action: "start", room: "design-review" },
    }),
    { kind: "list", prefillRoom: "design-review", focusStart: true },
  );
});

test("selectMeetingsView: action without room falls back to plain list", () => {
  assert.deepEqual(
    selectMeetingsView({
      hasCapability: true,
      isCapabilityLoading: false,
      deepLink: { action: "join" },
    }),
    { kind: "list", focusStart: false },
  );
});

test("isHostingSetupError: subscription + pending-invoice kinds only", () => {
  assert.equal(
    isHostingSetupError(new MeetingError("subscription_required", 402, "x")),
    true,
  );
  assert.equal(
    isHostingSetupError(new MeetingError("subscription_expired", 402, "x")),
    true,
  );
  assert.equal(
    isHostingSetupError(new MeetingError("pending_invoice", 409, "x")),
    true,
  );
  assert.equal(
    isHostingSetupError(new MeetingError("provider_unavailable", 503, "x")),
    false,
  );
  assert.equal(isHostingSetupError(new Error("nope")), false);
});

test("isRoomLive: participant count gate", () => {
  assert.equal(isRoomLive(undefined), false);
  assert.equal(isRoomLive(0), false);
  assert.equal(isRoomLive(2), true);
});
