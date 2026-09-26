import assert from "node:assert/strict";
import test from "node:test";

// Regression: the poof sound must be warmed through the Web Audio API, never
// by constructing an HTMLAudioElement at mount time. Constructing an
// HTMLAudioElement and calling `load()` makes the renderer synchronously wait
// on the media pipeline (`selectMediaResource` ->
// `RemoteMediaPlayerMIMETypeCache::supportsTypeAndCodecs`) before the relay
// TCP connection is up, which deadlocks the app on macOS 27 beta. The
// HTMLAudioElement fallback is constructed only after an actual user-triggered
// Web Audio playback failure.
test("warmPoofAudio warms Web Audio and constructs zero HTMLAudioElements", async () => {
  let audioConstructed = 0;
  let audioContextConstructed = 0;
  let bufferSourceStarted = 0;

  class FakeBufferSource {
    constructor() {
      this.buffer = null;
    }
    connect() {}
    start() {
      bufferSourceStarted += 1;
    }
  }

  class FakeAudioContext {
    constructor() {
      this.state = "running";
      this.destination = {};
      audioContextConstructed += 1;
    }
    createBufferSource() {
      return new FakeBufferSource();
    }
    createGain() {
      return { gain: { value: 0 }, connect() {} };
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
  const priorImage = globalThis.Image;

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
  };
  globalThis.Image = class {};

  try {
    // Fresh module instance so the module-level AudioContext/buffer caches
    // pick up the mocks installed above. (No query string: the test-loader's
    // `.tsx` hook matches on `url.endsWith(".tsx")`, so a `?query` suffix
    // would bypass it and fail with ERR_UNKNOWN_FILE_EXTENSION.)
    const mod = await import(`./PoofBurstProvider.tsx`);

    mod.warmPoofAudio();
    await new Promise((resolve) => setTimeout(resolve, 0));

    assert.equal(
      audioConstructed,
      0,
      "warmPoofAudio must not construct an HTMLAudioElement",
    );
    assert.equal(
      audioContextConstructed,
      1,
      "warmPoofAudio must warm the Web Audio buffer via an AudioContext",
    );
    assert.equal(bufferSourceStarted, 0, "warming must not start playback");
  } finally {
    globalThis.AudioContext = priorAudioContext;
    globalThis.fetch = priorFetch;
    globalThis.Audio = priorAudio;
    globalThis.Image = priorImage;
  }
});
