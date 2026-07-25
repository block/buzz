import assert from "node:assert/strict";
import test from "node:test";

import { createPoofAudioPlayer } from "./poofAudioLifecycle.ts";

function createScheduler() {
  let nextId = 1;
  const callbacks = new Map();

  return {
    clearTimeout(id) {
      callbacks.delete(id);
    },
    pendingCount() {
      return callbacks.size;
    },
    runAll() {
      const pending = [...callbacks.values()];
      callbacks.clear();
      for (const callback of pending) callback();
    },
    setTimeout(callback) {
      const id = nextId;
      nextId += 1;
      callbacks.set(id, callback);
      return id;
    },
  };
}

function createAudioHarness({
  deferSuspend = false,
  initialState = "running",
  resumeError = null,
  startError = null,
} = {}) {
  const calls = {
    fallback: 0,
    resume: 0,
    suspend: 0,
  };
  const sources = [];
  const gains = [];
  let state = initialState;
  let resolveDeferredSuspend = null;
  const deferredSuspend = deferSuspend
    ? new Promise((resolve) => {
        resolveDeferredSuspend = () => {
          state = "suspended";
          resolve();
        };
      })
    : null;

  const context = {
    createBufferSource() {
      const source = {
        buffer: null,
        connectedTo: null,
        disconnectCalls: 0,
        onended: null,
        startCalls: 0,
        startError,
        connect(target) {
          this.connectedTo = target;
        },
        disconnect() {
          this.disconnectCalls += 1;
        },
        finish() {
          this.onended?.();
        },
        start() {
          this.startCalls += 1;
          if (this.startError) throw this.startError;
        },
      };
      sources.push(source);
      return source;
    },
    createGain() {
      const gain = {
        connectedTo: null,
        disconnectCalls: 0,
        gain: { value: 0 },
        connect(target) {
          this.connectedTo = target;
        },
        disconnect() {
          this.disconnectCalls += 1;
        },
      };
      gains.push(gain);
      return gain;
    },
    destination: {},
    resume() {
      calls.resume += 1;
      if (resumeError) return Promise.reject(resumeError);
      state = "running";
      return Promise.resolve();
    },
    get state() {
      return state;
    },
    suspend() {
      calls.suspend += 1;
      if (deferredSuspend) return deferredSuspend;
      state = "suspended";
      return Promise.resolve();
    },
  };

  return {
    buffer: {},
    calls,
    context,
    fallback() {
      calls.fallback += 1;
    },
    gains,
    resolveSuspend() {
      resolveDeferredSuspend?.();
    },
    sources,
  };
}

function createPlayer(scheduler) {
  return createPoofAudioPlayer({
    clearTimeout: scheduler.clearTimeout,
    idleDelayMs: 1_500,
    setTimeout: scheduler.setTimeout,
  });
}

test("disconnects finished source and gain nodes, then suspends when idle", async () => {
  const scheduler = createScheduler();
  const audio = createAudioHarness();
  const player = createPlayer(scheduler);

  player.play(audio.context, audio.buffer, audio.fallback);
  audio.sources[0].finish();

  assert.equal(audio.sources[0].disconnectCalls, 1);
  assert.equal(audio.gains[0].disconnectCalls, 1);
  assert.equal(scheduler.pendingCount(), 1);
  assert.equal(audio.calls.suspend, 0);

  scheduler.runAll();
  await Promise.resolve();
  assert.equal(audio.calls.suspend, 1);
});

test("waits for every overlapping playback before scheduling suspension", async () => {
  const scheduler = createScheduler();
  const audio = createAudioHarness();
  const player = createPlayer(scheduler);

  player.play(audio.context, audio.buffer, audio.fallback);
  player.play(audio.context, audio.buffer, audio.fallback);

  audio.sources[0].finish();
  assert.equal(scheduler.pendingCount(), 0);

  audio.sources[1].finish();
  assert.equal(scheduler.pendingCount(), 1);
  scheduler.runAll();
  await Promise.resolve();
  assert.equal(audio.calls.suspend, 1);
});

test("new playback cancels the pending idle suspension", async () => {
  const scheduler = createScheduler();
  const audio = createAudioHarness();
  const player = createPlayer(scheduler);

  player.play(audio.context, audio.buffer, audio.fallback);
  audio.sources[0].finish();
  assert.equal(scheduler.pendingCount(), 1);

  player.play(audio.context, audio.buffer, audio.fallback);
  assert.equal(scheduler.pendingCount(), 0);

  audio.sources[1].finish();
  scheduler.runAll();
  await Promise.resolve();
  assert.equal(audio.calls.suspend, 1);
});

test("stale suspend completion resumes a newer active playback", async () => {
  const scheduler = createScheduler();
  const audio = createAudioHarness({ deferSuspend: true });
  const player = createPlayer(scheduler);

  player.play(audio.context, audio.buffer, audio.fallback);
  audio.sources[0].finish();
  scheduler.runAll();
  assert.equal(audio.calls.suspend, 1);

  player.play(audio.context, audio.buffer, audio.fallback);
  assert.equal(audio.sources[1].startCalls, 0);

  audio.resolveSuspend();
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(audio.calls.resume, 1);
  assert.equal(audio.sources[1].startCalls, 1);
  assert.equal(audio.context.state, "running");
});

test("graph creation failure re-arms idle suspension after cancelling it", async () => {
  const scheduler = createScheduler();
  const audio = createAudioHarness();
  const player = createPlayer(scheduler);

  player.play(audio.context, audio.buffer, audio.fallback);
  audio.sources[0].finish();
  assert.equal(scheduler.pendingCount(), 1);

  audio.context.createGain = () => {
    throw new Error("gain creation failed");
  };
  player.play(audio.context, audio.buffer, audio.fallback);

  assert.equal(audio.calls.fallback, 1);
  assert.equal(audio.sources[1].disconnectCalls, 1);
  assert.equal(scheduler.pendingCount(), 1);

  scheduler.runAll();
  await Promise.resolve();
  assert.equal(audio.calls.suspend, 1);
});

test("resume failure cleans up the graph and falls back", async () => {
  const scheduler = createScheduler();
  const audio = createAudioHarness({
    initialState: "suspended",
    resumeError: new Error("resume blocked"),
  });
  const player = createPlayer(scheduler);

  player.play(audio.context, audio.buffer, audio.fallback);
  await Promise.resolve();
  await Promise.resolve();

  assert.equal(audio.calls.resume, 1);
  assert.equal(audio.calls.fallback, 1);
  assert.equal(audio.sources[0].startCalls, 0);
  assert.equal(audio.sources[0].disconnectCalls, 1);
  assert.equal(audio.gains[0].disconnectCalls, 1);
});

test("start failure cleans up the graph and falls back", () => {
  const scheduler = createScheduler();
  const audio = createAudioHarness({ startError: new Error("start failed") });
  const player = createPlayer(scheduler);

  player.play(audio.context, audio.buffer, audio.fallback);

  assert.equal(audio.calls.fallback, 1);
  assert.equal(audio.sources[0].disconnectCalls, 1);
  assert.equal(audio.gains[0].disconnectCalls, 1);
});
