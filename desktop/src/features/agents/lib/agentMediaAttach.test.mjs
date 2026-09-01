/**
 * The lip-sync invariant.
 *
 * Provenance: the first audio fix gave the voice its own hidden `<audio>`
 * element, and the avatar's lips drifted against the speech — two elements
 * play from two independent clocks. LiveKit's `attach` merges tracks into the
 * element's existing `MediaStream`, so one element is one clock. These pin
 * "both tracks, one element" so the drift cannot come back quietly.
 *
 * What they do not pin, stated plainly because the gap is easy to miss: the
 * element below is a stand-in and both attachers are spies, so LiveKit never
 * runs. The SDK behaviour that caused the original silence — `attachToElement`
 * setting `element.muted` from the stream's own audio-track count — is
 * unverified here, as is everything in `useAgentMediaRoom` and
 * `AgentMediaSurface`: autoplay recovery, teardown, microphone state,
 * reconnection. Pinning any of it needs a real media element and the real SDK,
 * which the desktop unit lane (node, no DOM) cannot give and the mock-bridge
 * Playwright lane has no room to join. So the rule is pinned; the SDK's own
 * muting is covered only by the live run that exposed it.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { attachSessionTracks } from "./agentMediaAttach.ts";

/** A stand-in element; the rule never touches the DOM. */
const ELEMENT = { tagName: "VIDEO" };

/** An attacher that records the element it was given and its detach calls. */
function spy() {
  const calls = [];
  const detached = [];
  const attach = (element) => {
    calls.push(element);
    return () => detached.push(element);
  };
  return { attach, calls, detached };
}

test("attachSessionTracks puts audio and video on the same element", () => {
  const video = spy();
  const audio = spy();

  attachSessionTracks(ELEMENT, {
    attachAudio: audio.attach,
    attachVideo: video.attach,
  });

  assert.deepEqual(video.calls, [ELEMENT]);
  assert.deepEqual(audio.calls, [ELEMENT]);
  assert.equal(
    audio.calls[0],
    video.calls[0],
    "one element means one playback clock — this is the lip-sync guarantee",
  );
});

test("attachSessionTracks attaches video before audio", () => {
  // The SDK decides `element.muted` from the stream's audio tracks as each one
  // attaches, so the video track must be there first for both to be seen.
  const order = [];
  attachSessionTracks(ELEMENT, {
    attachAudio: () => {
      order.push("audio");
      return () => {};
    },
    attachVideo: () => {
      order.push("video");
      return () => {};
    },
  });
  assert.deepEqual(order, ["video", "audio"]);
});

test("attachSessionTracks detaches everything it attached", () => {
  const video = spy();
  const audio = spy();

  const detach = attachSessionTracks(ELEMENT, {
    attachAudio: audio.attach,
    attachVideo: video.attach,
  });
  detach();

  assert.deepEqual(video.detached, [ELEMENT], "video detached");
  assert.deepEqual(audio.detached, [ELEMENT], "audio detached");
});

test("attachSessionTracks handles a session with no audio yet", () => {
  // Tracks arrive independently: the face may render a beat before the voice
  // is subscribed, and that must not throw or leave a detach missing.
  const video = spy();

  const detach = attachSessionTracks(ELEMENT, {
    attachAudio: null,
    attachVideo: video.attach,
  });
  detach();

  assert.deepEqual(video.calls, [ELEMENT]);
  assert.deepEqual(video.detached, [ELEMENT]);
});

test("attachSessionTracks is a no-op when no track is available", () => {
  const detach = attachSessionTracks(ELEMENT, {
    attachAudio: null,
    attachVideo: null,
  });
  assert.doesNotThrow(detach);
});
