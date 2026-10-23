import assert from "node:assert/strict";
import test from "node:test";
import {
  acquireHuddleCamera,
  listHuddleCameras,
  startCameraFramePump,
} from "./huddleCamera.ts";
import { CameraController } from "./cameraController.ts";

const share = {
  session_id: "22222222-2222-4222-8222-222222222222",
  started_at: 1000,
  label: "Camera",
};
function captured() {
  const track = {
    stopped: false,
    stop() {
      this.stopped = true;
    },
  };
  return { track, getTracks: () => [track], getVideoTracks: () => [track] };
}
function devices(results) {
  const calls = [];
  return {
    calls,
    getSupportedConstraints: () => ({
      width: true,
      height: true,
      frameRate: true,
    }),
    getUserMedia: async (request) => {
      calls.push(request);
      const result = results.shift();
      if (result instanceof Error) throw result;
      return result;
    },
  };
}

test("camera detection accepts redacted devices without opening them", async () => {
  const found = await listHuddleCameras({
    enumerateDevices: async () => [
      { kind: "audioinput", deviceId: "mic", label: "Mic" },
      { kind: "videoinput", deviceId: "", label: "" },
    ],
  });
  assert.deepEqual(found, [{ deviceId: "", label: "Camera 1" }]);
});

test("camera capture is video-only and retains an explicitly selected device", async () => {
  const stream = captured();
  const input = devices([stream]);
  assert.equal(await acquireHuddleCamera(input, "usb", () => true), stream);
  assert.deepEqual(input.calls[0], {
    audio: false,
    video: {
      width: { ideal: 1280 },
      height: { ideal: 720 },
      frameRate: { ideal: 30, max: 30 },
      deviceId: { exact: "usb" },
    },
  });
});

test("constraint recovery retries once without substituting another camera", async () => {
  const error = Object.assign(new Error("Invalid constraint"), {
    name: "OverconstrainedError",
  });
  const input = devices([error, captured()]);
  await acquireHuddleCamera(input, "usb", () => true);
  assert.deepEqual(input.calls[1], {
    audio: false,
    video: { deviceId: { exact: "usb" } },
  });
});

test("permission, missing-device, and busy-device failures propagate without retries", async () => {
  for (const name of [
    "NotAllowedError",
    "NotFoundError",
    "NotReadableError",
    "AbortError",
  ]) {
    const error = Object.assign(new Error(name), { name });
    const input = devices([error]);
    await assert.rejects(
      acquireHuddleCamera(input, "", () => true),
      error,
    );
    assert.equal(input.calls.length, 1);
  }
});

test("late camera permission grants release tracks and cannot begin publishing", async () => {
  let alive = true;
  const stream = captured();
  const input = devices([stream]);
  input.getUserMedia = async () => {
    alive = false;
    return stream;
  };
  await assert.rejects(
    acquireHuddleCamera(input, "", () => alive),
    /cancelled/,
  );
  assert.equal(stream.track.stopped, true);
});

function harness(overrides = {}) {
  const calls = [];
  const stream = captured();
  const owner = new CameraController({
    current: () => true,
    reserve: async () => {
      calls.push("reserve");
      return share;
    },
    retire: async () => {
      calls.push("retire");
    },
    notifyStop: async () => {
      calls.push("notify");
    },
    acquire: async () => {
      calls.push("acquire");
      return stream;
    },
    pump: async () => {
      calls.push("pump");
      return () => calls.push("stop-frames");
    },
    publish: () => {
      calls.push("publish");
      return () => calls.push("stop-publisher");
    },
    changed: () => {},
    failed: (error) => {
      throw error;
    },
    ...overrides,
  });
  return { owner, calls, stream };
}

test("camera remains off until an explicit action, then stops the LED/track before IPC", async () => {
  const h = harness();
  assert.deepEqual(h.calls, []);
  await h.owner.start("room", "", "Camera");
  assert.deepEqual(h.calls, ["reserve", "acquire", "pump", "publish"]);
  await h.owner.stop();
  assert.equal(h.stream.track.stopped, true);
  assert.deepEqual(h.calls.slice(4), [
    "stop-frames",
    "stop-publisher",
    "retire",
    "notify",
  ]);
});

test("one pending permission request prevents a second acquisition", async () => {
  let resolve;
  const h = harness({
    acquire: () =>
      new Promise((done) => {
        resolve = done;
      }),
  });
  const first = h.owner.start("room", "", "Camera");
  await new Promise((done) => setImmediate(done));
  await assert.rejects(h.owner.start("room", "other", "Other"), /in progress/);
  await h.owner.stop();
  resolve(h.stream);
  await first;
  assert.equal(h.stream.track.stopped, true);
  assert.ok(!h.calls.includes("publish"));
});

test("a native reservation completing after stop is retired without opening a camera", async () => {
  let resolve;
  const h = harness({
    reserve: () =>
      new Promise((done) => {
        resolve = done;
      }),
  });
  const pending = h.owner.start("room", "", "Camera");
  await new Promise((done) => setImmediate(done));
  await h.owner.stop();
  resolve(share);
  await pending;
  assert.deepEqual(h.calls, ["retire"]);
});

test("a camera switch releases the old stream and publisher before another reservation", async () => {
  const h = harness();
  await h.owner.start("room", "one", "One");
  await h.owner.start("room", "two", "Two");
  const oldStop = h.calls.indexOf("stop-publisher");
  const secondReserve = h.calls.lastIndexOf("reserve");
  assert.ok(oldStop < secondReserve);
  assert.equal(h.stream.track.stopped, true);
  await h.owner.stop();
});

test("scope changes during acquisition cannot leak frames into a new room", async () => {
  let current = true;
  const h = harness({
    current: () => current,
    acquire: async () => {
      current = false;
      return captured();
    },
  });
  await h.owner.start("room", "", "Camera");
  assert.ok(!h.calls.includes("pump"));
  await h.owner.stop();
});

function pumpHarness(t) {
  const previous = {
    document: globalThis.document,
    window: globalThis.window,
    Worker: globalThis.Worker,
    performance: Object.getOwnPropertyDescriptor(globalThis, "performance"),
  };
  let worker;
  globalThis.Worker = class {
    sent = [];
    terminated = false;
    constructor() {
      worker = this;
    }
    postMessage(message) {
      this.sent.push(message);
    }
    terminate() {
      this.terminated = true;
    }
    tick() {
      const request = this.sent.at(-1);
      this.onmessage?.({ data: request.id });
    }
  };
  const video = {
    frames: 1,
    currentTime: 0.1,
    readyState: 2,
    videoWidth: 32,
    videoHeight: 16,
    play: async () => {},
    pause() {
      this.paused = true;
    },
    load() {},
    getVideoPlaybackQuality() {
      return { totalVideoFrames: this.frames };
    },
  };
  let now = 0;
  const timers = [];
  globalThis.window = {
    setTimeout: (_callback, delay) => {
      timers.push(delay);
      return 1;
    },
    clearTimeout: () => {},
  };
  Object.defineProperty(globalThis, "performance", {
    configurable: true,
    value: { now: () => now },
  });
  globalThis.document = {
    createElement: (tag) =>
      tag === "video"
        ? video
        : {
            width: 0,
            height: 0,
            getContext: () => ({
              drawImage() {},
              getImageData: () => ({
                data: new Uint8ClampedArray(32 * 16 * 4),
              }),
            }),
          },
  };
  const track = { addEventListener() {}, removeEventListener() {} };
  const stream = { getVideoTracks: () => [track] };
  t.after(() => {
    globalThis.document = previous.document;
    globalThis.window = previous.window;
    globalThis.Worker = previous.Worker;
    Object.defineProperty(globalThis, "performance", previous.performance);
  });
  return {
    get worker() {
      return worker;
    },
    video,
    stream,
    timers,
    advance: (value) => {
      now = value;
    },
  };
}

test("the production camera pump keeps moving without foreground window timers", async (t) => {
  const h = pumpHarness(t);
  let frames = 0;
  const stop = await startCameraFramePump({
    stream: h.stream,
    isCurrent: () => true,
    submit: async () => frames++,
    fail: assert.fail,
  });
  t.after(stop);
  for (let frame = 2; frame <= 30; frame++) {
    h.video.frames = frame;
    h.advance(frame * 33);
    h.worker.tick();
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.equal(frames, 30);
  assert.deepEqual(
    h.timers,
    [10_000],
    "only startup readiness uses a window timeout",
  );
  stop();
  assert.equal(h.worker.terminated, true);
  assert.equal(h.video.srcObject, null);
});

test("the camera pump cannot keep publishing the same stalled frame forever", async (t) => {
  const h = pumpHarness(t);
  let frames = 0;
  const failures = [];
  const stop = await startCameraFramePump({
    stream: h.stream,
    isCurrent: () => true,
    submit: async () => frames++,
    fail: (error) => failures.push(error.message),
  });
  t.after(stop);
  h.advance(1000);
  h.worker.tick();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    frames,
    1,
    "unchanged decoded frames are not snapshots to resend",
  );
  h.advance(6000);
  h.worker.tick();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(failures, ["Camera stopped producing video"]);
  assert.equal(h.worker.terminated, true);
  assert.equal(h.video.srcObject, null);
});
