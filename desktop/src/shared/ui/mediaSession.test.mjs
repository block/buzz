import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

// macOS gives the hardware play/pause key to the app holding the Now Playing
// session. WKWebView claims one for Buzz on the first audible <video> and
// keeps it until the element itself is torn down — dropping the element from
// the DOM is not enough. These tests pin the teardown contract that hands the
// key back: pause, detach the source, clear the metadata we published.

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const paused = [];
const loaded = [];
let mediaSession;

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  // jsdom leaves the playback methods unimplemented; record the calls instead.
  dom.window.HTMLMediaElement.prototype.pause = function pause() {
    paused.push(this);
  };
  dom.window.HTMLMediaElement.prototype.load = function load() {
    loaded.push(this);
  };
  globalThis.MediaMetadata = class MediaMetadata {
    constructor(init) {
      Object.assign(this, init);
    }
  };
  // Neither node's built-in `navigator` nor jsdom's implements the Media
  // Session API, so graft a stub onto jsdom's and make it the global.
  Object.defineProperty(dom.window.navigator, "mediaSession", {
    configurable: true,
    value: { metadata: null, playbackState: "none" },
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  mediaSession = globalThis.navigator.mediaSession;
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  paused.length = 0;
  loaded.length = 0;
  mediaSession.metadata = null;
  mediaSession.playbackState = "none";
});

after(() => dom.window.close());

async function renderVideo({ strict = false } = {}) {
  const React = await import("react");
  const { act, render } = await import("@testing-library/react");
  const { useReleasingVideoRef } = await import("./mediaSession.ts");
  const released = [];

  function Player() {
    const attach = useReleasingVideoRef({
      onRelease: (video) => released.push(video.currentTime),
    });
    return React.createElement("video", {
      ref: attach,
      src: "https://relay.example/media/clip.mp4",
    });
  }

  const tree = React.createElement(Player);
  const result = render(
    strict ? React.createElement(React.StrictMode, null, tree) : tree,
  );
  await act(async () => {});
  return {
    act,
    released,
    unmount: result.unmount,
    video: result.container.querySelector("video"),
  };
}

test("unmounting a player releases the media keys it claimed", async () => {
  const { claimMediaSession } = await import("./mediaSession.ts");
  const { act, video, unmount } = await renderVideo();

  claimMediaSession(video, { artist: "engineering", title: "clip.mp4" });
  assert.equal(mediaSession.metadata.title, "clip.mp4");
  assert.equal(mediaSession.playbackState, "playing");

  await act(async () => unmount());

  // Pause alone leaves the session alive — WebKit only destroys it once the
  // source is detached, which is what returns the key to Music.
  assert.ok(paused.includes(video), "the element was paused");
  assert.equal(video.getAttribute("src"), null, "the source was detached");
  assert.ok(loaded.includes(video), "load() reset the element");
  assert.equal(mediaSession.metadata, null);
  assert.equal(mediaSession.playbackState, "none");
});

test("release runs before the source is detached, so the position is still readable", async () => {
  const { released, act, unmount } = await renderVideo();

  await act(async () => unmount());

  assert.deepEqual(released, [0]);
});

test("a bystander player's teardown leaves the playing video's session alone", async () => {
  const { claimMediaSession, releaseVideoElement } = await import(
    "./mediaSession.ts"
  );
  const playing = dom.window.document.createElement("video");
  const bystander = dom.window.document.createElement("video");

  claimMediaSession(playing, { title: "review.mp4" });
  // An off-screen timeline row unmounting while the review overlay plays.
  releaseVideoElement(bystander);

  assert.equal(mediaSession.metadata.title, "review.mp4");
  assert.equal(mediaSession.playbackState, "playing");
});

test("StrictMode's simulated remount leaves the source attached", async () => {
  const { video } = await renderVideo({ strict: true });

  // StrictMode runs the ref cleanup and re-attaches the same element without
  // re-applying props; a release that did not restore the source would blank
  // every video in development.
  assert.equal(
    video.getAttribute("src"),
    "https://relay.example/media/clip.mp4",
  );
});
