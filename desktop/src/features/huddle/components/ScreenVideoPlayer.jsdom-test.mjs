import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import * as React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { ScreenVideoPlayer } from "./ScreenVideoPlayer.tsx";

const media = window.HTMLMediaElement.prototype;
const original = new Map(
  ["play", "pause", "load"].map((name) => [name, media[name]]),
);
const frameApi = window.HTMLVideoElement.prototype;
const originalRequest = frameApi.requestVideoFrameCallback;
const originalCancel = frameApi.cancelVideoFrameCallback;
let callbacks;
let nextFrame;
let root;
let container;
let video;
let ranges;
let seeking;
let unsubscribed;

beforeEach(async () => {
  media.play = async () => {};
  media.pause = () => {};
  media.load = () => {};
  callbacks = new Map();
  nextFrame = 0;
  frameApi.requestVideoFrameCallback = (callback) => {
    const id = ++nextFrame;
    callbacks.set(id, callback);
    return id;
  };
  frameApi.cancelVideoFrameCallback = (id) => callbacks.delete(id);
  ranges = [[0, 1.5]];
  seeking = false;
  unsubscribed = false;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(
        TooltipProvider,
        null,
        React.createElement(ScreenVideoPlayer, {
          stream: {
            subscribe: () => () => {
              unsubscribed = true;
            },
          },
          label: "Live camera",
          camera: true,
        }),
      ),
    );
  });
  video = container.querySelector("video");
  Object.defineProperties(video, {
    readyState: { configurable: true, get: () => 4 },
    seeking: { configurable: true, get: () => seeking },
    buffered: {
      configurable: true,
      get: () => ({
        length: ranges.length,
        start: (index) => ranges[index][0],
        end: (index) => ranges[index][1],
      }),
    },
  });
  video.currentTime = 0.1;
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  for (const [name, implementation] of original) media[name] = implementation;
  if (originalRequest) frameApi.requestVideoFrameCallback = originalRequest;
  else delete frameApi.requestVideoFrameCallback;
  if (originalCancel) frameApi.cancelVideoFrameCallback = originalCancel;
  else delete frameApi.cancelVideoFrameCallback;
});

test("the production player catches up a one-second backlog after media is appended", () => {
  video.dispatchEvent(new Event("progress"));
  assert.ok(video.currentTime >= 1.1, "old video must not stay on screen");
  assert.ok(1.5 - video.currentTime <= 0.35);
});

test("initial keyframe-history playback starts near the live edge", () => {
  ranges = [[0, 0.95]];
  video.currentTime = 0;
  video.dispatchEvent(new Event("loadeddata"));
  assert.ok(video.currentTime >= 0.6);
});

test("normal transport jitter does not cause repeated playback seeks", () => {
  ranges = [[0, 1.4]];
  video.currentTime = 1.1;
  video.dispatchEvent(new Event("timeupdate"));
  assert.equal(video.currentTime, 1.1);
});

test("small steady-state drift drains without repeatedly seeking the native decoder", () => {
  ranges = [[0, 1.4]];
  video.currentTime = 1.1;
  video.dispatchEvent(new Event("loadeddata"));
  ranges = [[0, 1.8]];
  video.dispatchEvent(new Event("progress"));
  assert.equal(video.currentTime, 1.1);
  assert.equal(video.playbackRate, 1.25);
  video.currentTime = 1.6;
  video.dispatchEvent(new Event("timeupdate"));
  assert.equal(video.playbackRate, 1);
});

test("large steady-state backlogs still skip straight to live video", () => {
  ranges = [[0, 1.4]];
  video.currentTime = 1.1;
  video.dispatchEvent(new Event("loadeddata"));
  ranges = [[0, 3]];
  video.dispatchEvent(new Event("progress"));
  assert.ok(video.currentTime >= 2.6);
  assert.equal(video.playbackRate, 1);
});

test("a decoder reset restores startup catch-up and clears accelerated playback", () => {
  ranges = [[0, 1.4]];
  video.currentTime = 1.1;
  video.dispatchEvent(new Event("loadeddata"));
  ranges = [[0, 1.8]];
  video.dispatchEvent(new Event("progress"));
  assert.equal(video.playbackRate, 1.25);
  video.dispatchEvent(new Event("emptied"));
  assert.equal(video.playbackRate, 1);
  ranges = [[0, 0.9]];
  video.currentTime = 0;
  video.dispatchEvent(new Event("loadeddata"));
  assert.ok(video.currentTime >= 0.6);
});

test("retiring an accelerated player restores its normal playback rate", async () => {
  ranges = [[0, 1.4]];
  video.currentTime = 1.1;
  video.dispatchEvent(new Event("loadeddata"));
  ranges = [[0, 1.8]];
  video.dispatchEvent(new Event("progress"));
  assert.equal(video.playbackRate, 1.25);
  await act(async () => root.unmount());
  assert.equal(video.playbackRate, 1);
});

test("a pending seek is allowed to finish before the next live correction", () => {
  seeking = true;
  video.dispatchEvent(new Event("progress"));
  assert.equal(video.currentTime, 0.1);
  seeking = false;
  video.dispatchEvent(new Event("seeked"));
  assert.ok(video.currentTime >= 1.1);
});

test("catch-up uses the latest buffered range rather than seeking into a hole", () => {
  ranges = [
    [0, 0.2],
    [5, 5.6],
  ];
  video.dispatchEvent(new Event("progress"));
  assert.ok(video.currentTime >= 5 && video.currentTime < 5.6);
});

test("returning to a hidden viewer's window catches up its buffered video", () => {
  document.dispatchEvent(new Event("visibilitychange"));
  assert.ok(video.currentTime >= 1.1);
});

test("retiring a player releases its playback correction listeners", async () => {
  await act(async () => root.unmount());
  assert.equal(unsubscribed, true);
  video.currentTime = 0.1;
  video.dispatchEvent(new Event("progress"));
  document.dispatchEvent(new Event("visibilitychange"));
  assert.equal(video.currentTime, 0.1);
});

test("presented frames correct playback without waiting for another relay packet", () => {
  assert.equal(callbacks.size, 1);
  const [id, callback] = [...callbacks][0];
  callbacks.delete(id);
  callback();
  assert.ok(video.currentTime >= 1.1);
  assert.equal(
    callbacks.size,
    1,
    "one presentation callback remains in flight",
  );
});

test("late presentation callbacks cannot revive a retired player", async () => {
  const callback = [...callbacks.values()][0];
  await act(async () => root.unmount());
  assert.equal(callbacks.size, 0);
  callback();
  assert.equal(callbacks.size, 0);
});

test("a native seek failure stays visible with the explicit playback retry", async () => {
  Object.defineProperty(video, "currentTime", {
    configurable: true,
    get: () => 0.1,
    set: () => {
      throw new Error("Native seek rejected");
    },
  });
  await act(async () => video.dispatchEvent(new Event("progress")));
  assert.match(
    container.querySelector('[role="alert"]').textContent,
    /Native seek rejected/,
  );
  assert.ok(
    container.querySelector('button[aria-label="Retry video playback"]'),
  );
  assert.equal(callbacks.size, 0);
  Object.defineProperty(video, "currentTime", {
    configurable: true,
    value: 0.1,
    writable: true,
  });
  await act(async () =>
    container
      .querySelector('button[aria-label="Retry video playback"]')
      .click(),
  );
  assert.equal(container.querySelector('[role="alert"]'), null);
  assert.ok(video.currentTime >= 1.1);
  assert.equal(callbacks.size, 1);
});
