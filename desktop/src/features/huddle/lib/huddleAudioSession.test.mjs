import assert from "node:assert/strict";
import { describe, test } from "node:test";

import {
  shouldClaimHuddleMedia,
  shouldEndRustSessionOnAudioOwnerUnmount,
  shouldOwnHuddleAudioSession,
  shouldSetupMediaOnHuddleStart,
} from "./huddleAudioSession.ts";

describe("huddleAudioSession", () => {
  test("companion room owns audio; main does not", () => {
    assert.equal(shouldOwnHuddleAudioSession(true), true);
    assert.equal(shouldOwnHuddleAudioSession(false), false);
  });

  test("only the audio owner sets up media on start/join", () => {
    assert.equal(shouldSetupMediaOnHuddleStart(true), true);
    assert.equal(shouldSetupMediaOnHuddleStart(false), false);
  });

  test("audio-owner unmount releases mic but does not end Rust session", () => {
    assert.equal(shouldEndRustSessionOnAudioOwnerUnmount(), false);
  });

  test("companion claims media for connected/active sessions once", () => {
    assert.equal(
      shouldClaimHuddleMedia({
        ownsAudioSession: true,
        phase: "active",
        ephemeralChannelId: "eph",
        alreadyConnected: false,
        claimInFlight: false,
      }),
      true,
    );
    assert.equal(
      shouldClaimHuddleMedia({
        ownsAudioSession: true,
        phase: "connected",
        ephemeralChannelId: "eph",
        alreadyConnected: false,
        claimInFlight: false,
      }),
      true,
    );
    assert.equal(
      shouldClaimHuddleMedia({
        ownsAudioSession: false,
        phase: "active",
        ephemeralChannelId: "eph",
        alreadyConnected: false,
        claimInFlight: false,
      }),
      false,
    );
    assert.equal(
      shouldClaimHuddleMedia({
        ownsAudioSession: true,
        phase: "idle",
        ephemeralChannelId: "eph",
        alreadyConnected: false,
        claimInFlight: false,
      }),
      false,
    );
    assert.equal(
      shouldClaimHuddleMedia({
        ownsAudioSession: true,
        phase: "active",
        ephemeralChannelId: null,
        alreadyConnected: false,
        claimInFlight: false,
      }),
      false,
    );
    assert.equal(
      shouldClaimHuddleMedia({
        ownsAudioSession: true,
        phase: "active",
        ephemeralChannelId: "eph",
        alreadyConnected: true,
        claimInFlight: false,
      }),
      false,
    );
    assert.equal(
      shouldClaimHuddleMedia({
        ownsAudioSession: true,
        phase: "active",
        ephemeralChannelId: "eph",
        alreadyConnected: false,
        claimInFlight: true,
      }),
      false,
    );
  });
});
