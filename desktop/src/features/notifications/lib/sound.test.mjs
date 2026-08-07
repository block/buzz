import assert from "node:assert/strict";
import test from "node:test";

import { shouldPlayNotificationSound } from "./sound.ts";

test("silences notifications from Huddle backing channels", () => {
  const silentChannelIds = new Set(["active-huddle"]);

  assert.equal(
    shouldPlayNotificationSound("active-huddle", silentChannelIds),
    false,
  );
  assert.equal(
    shouldPlayNotificationSound("ordinary-channel", silentChannelIds),
    true,
  );
  assert.equal(shouldPlayNotificationSound(null, silentChannelIds), true);
});

// Regression: notification sounds must play through the Web Audio API, not an
// HTMLAudioElement. A playing HTMLMediaElement is adopted by the browser's
// Media Session, which binds the OS media keys (macOS Play/Pause) to it — the
// bug where pressing Play kept replaying the preview ping. Web Audio buffer
// sources are never captured by media keys.
test("playNotificationSound uses Web Audio and never an HTMLAudioElement when available", async () => {
  const started = [];
  let audioConstructed = 0;

  class FakeBufferSource {
    constructor() {
      this.buffer = null;
      this._ended = null;
    }
    connect() {}
    addEventListener(type, cb) {
      if (type === "ended") this._ended = cb;
    }
    start() {
      started.push(this);
    }
    stop() {
      this._ended?.();
    }
  }

  class FakeAudioContext {
    constructor() {
      this.state = "running";
      this.destination = {};
    }
    createBufferSource() {
      return new FakeBufferSource();
    }
    async decodeAudioData() {
      return { duration: 0.2 };
    }
    async resume() {
      this.state = "running";
    }
  }

  const priorAudioContext = globalThis.AudioContext;
  const priorFetch = globalThis.fetch;
  const priorAudio = globalThis.Audio;

  globalThis.AudioContext = FakeAudioContext;
  globalThis.fetch = async () => ({
    ok: true,
    status: 200,
    arrayBuffer: async () => new ArrayBuffer(8),
  });
  globalThis.Audio = class {
    constructor() {
      audioConstructed += 1;
    }
    play() {
      return Promise.resolve();
    }
    pause() {}
    addEventListener() {}
  };

  try {
    // Fresh module instance so the module-level AudioContext/buffer caches
    // pick up the mocks installed above.
    const mod = await import(`./sound.ts?webaudio=${Date.now()}`);

    // Warm the decoded-buffer cache, then play.
    mod.preloadNotificationSound("ping");
    await new Promise((resolve) => setTimeout(resolve, 0));

    const playback = mod.playNotificationSound("ping");
    assert.equal(started.length, 1, "expected a Web Audio buffer source start");
    assert.equal(
      audioConstructed,
      0,
      "must not construct an HTMLAudioElement on the Web Audio path",
    );

    let ended = false;
    playback.onEnded(() => {
      ended = true;
    });
    playback.stop();
    assert.equal(ended, true, "stop() must fire the onEnded callback");
  } finally {
    globalThis.AudioContext = priorAudioContext;
    globalThis.fetch = priorFetch;
    globalThis.Audio = priorAudio;
  }
});

test("playNotificationSound falls back to HTMLAudioElement without Web Audio", async () => {
  let audioConstructed = 0;

  const priorAudioContext = globalThis.AudioContext;
  const priorAudio = globalThis.Audio;

  // No AudioContext available -> fallback path.
  globalThis.AudioContext = undefined;
  globalThis.Audio = class {
    constructor() {
      audioConstructed += 1;
      this.currentTime = 0;
    }
    play() {
      return Promise.resolve();
    }
    pause() {}
    addEventListener() {}
  };

  try {
    const mod = await import(`./sound.ts?fallback=${Date.now()}`);
    const playback = mod.playNotificationSound("ping");
    assert.equal(
      audioConstructed,
      1,
      "fallback must construct exactly one HTMLAudioElement",
    );
    assert.equal(typeof playback.stop, "function");
    assert.equal(typeof playback.onEnded, "function");
  } finally {
    globalThis.AudioContext = priorAudioContext;
    globalThis.Audio = priorAudio;
  }
});
