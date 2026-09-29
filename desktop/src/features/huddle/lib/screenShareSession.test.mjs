import assert from "node:assert/strict";
import test from "node:test";

import { isShareBlockedByOther, livekitRoomName } from "./screenSharePolicy.ts";
import {
  acquireDisplayMedia,
  stopMediaStreamTracks,
} from "./screenShareMedia.ts";

test("livekitRoomName matches relay format", () => {
  assert.equal(
    livekitRoomName("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
    "huddle-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
  );
});

test("isShareBlockedByOther allows empty sharer", () => {
  assert.equal(
    isShareBlockedByOther({ selfPubkey: "aa", currentSharer: null }),
    false,
  );
});

test("isShareBlockedByOther blocks other sharer", () => {
  assert.equal(
    isShareBlockedByOther({ selfPubkey: "aa", currentSharer: "bb" }),
    true,
  );
});

test("isShareBlockedByOther allows self sharer", () => {
  assert.equal(
    isShareBlockedByOther({ selfPubkey: "Aa", currentSharer: "aa" }),
    false,
  );
});

function fakeTrack(kind = "video") {
  let stopped = false;
  return {
    kind,
    stop() {
      stopped = true;
    },
    get stopped() {
      return stopped;
    },
    addEventListener() {},
  };
}

test("stopMediaStreamTracks stops every track", () => {
  const a = fakeTrack("video");
  const b = fakeTrack("audio");
  stopMediaStreamTracks({ getTracks: () => [a, b] });
  assert.equal(a.stopped, true);
  assert.equal(b.stopped, true);
});

test("stopMediaStreamTracks tolerates null", () => {
  stopMediaStreamTracks(null);
  stopMediaStreamTracks(undefined);
});

test("acquireDisplayMedia returns stream with video track", async () => {
  const track = fakeTrack("video");
  const stream = {
    getVideoTracks: () => [track],
    getTracks: () => [track],
  };
  const original = globalThis.navigator;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      mediaDevices: {
        getDisplayMedia: async (opts) => {
          assert.deepEqual(opts, { video: true, audio: false });
          return stream;
        },
      },
    },
  });
  try {
    const got = await acquireDisplayMedia();
    assert.equal(got, stream);
  } finally {
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: original,
    });
  }
});

test("acquireDisplayMedia stops tracks when no video", async () => {
  const audio = fakeTrack("audio");
  const stream = {
    getVideoTracks: () => [],
    getTracks: () => [audio],
  };
  const original = globalThis.navigator;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      mediaDevices: {
        getDisplayMedia: async () => stream,
      },
    },
  });
  try {
    await assert.rejects(() => acquireDisplayMedia(), /no screen video track/);
    assert.equal(audio.stopped, true);
  } finally {
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: original,
    });
  }
});

test("screenShareSession source forces dual peer connection", async () => {
  const src = await import("node:fs").then((fs) =>
    fs.readFileSync(
      new URL("./screenShareSession.ts", import.meta.url),
      "utf8",
    ),
  );
  assert.match(src, /singlePeerConnection:\s*false/);
  assert.match(src, /acquireDisplayMedia/);
});
