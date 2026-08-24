import assert from "node:assert/strict";
import test from "node:test";

class FakeBufferSource extends EventTarget {
  buffer = null;
  connectedTo = null;
  started = false;
  stopped = false;

  connect(destination) {
    this.connectedTo = destination;
  }

  start() {
    this.started = true;
  }

  stop() {
    this.stopped = true;
    this.dispatchEvent(new Event("ended"));
  }

  end() {
    this.dispatchEvent(new Event("ended"));
  }
}

class FakeAudioContext {
  static instances = [];

  destination = { id: "destination" };
  state = "running";
  sources = [];
  decodeCalls = 0;
  resumeCalls = 0;

  constructor(options) {
    this.options = options;
    FakeAudioContext.instances.push(this);
  }

  async decodeAudioData(data) {
    this.decodeCalls += 1;
    return { data };
  }

  createBufferSource() {
    const source = new FakeBufferSource();
    this.sources.push(source);
    return source;
  }

  async resume() {
    this.resumeCalls += 1;
    this.state = "running";
  }
}

const fetchCalls = [];
globalThis.Audio = class {
  constructor() {
    throw new Error("notification sounds must not create HTML media elements");
  }
};
globalThis.AudioContext = FakeAudioContext;
const successfulFetch = async (url) => {
  fetchCalls.push(url);
  return {
    ok: true,
    arrayBuffer: async () => new ArrayBuffer(4),
  };
};
globalThis.fetch = successfulFetch;

const { playNotificationSound, shouldPlayNotificationSound } = await import(
  `./sound.ts?test=${Date.now()}`
);

async function flushPlayback() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

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

test("plays notification cues through Web Audio without HTML media", async () => {
  const playback = playNotificationSound("flutter");
  assert.ok(playback);

  await flushPlayback();

  assert.equal(FakeAudioContext.instances.length, 1);
  const context = FakeAudioContext.instances[0];
  assert.deepEqual(context.options, { latencyHint: "interactive" });
  assert.equal(context.sources.length, 1);
  assert.equal(context.sources[0].started, true);
  assert.equal(context.sources[0].connectedTo, context.destination);
  assert.equal(fetchCalls[0], "/sounds/flutter.mp3");
  context.sources[0].end();
});

test("caches decoded buffers and replaces only the same active cue", async () => {
  const context = FakeAudioContext.instances[0];
  const first = playNotificationSound("flutter");
  let firstEnded = 0;
  first.onEnded(() => {
    firstEnded += 1;
  });
  await flushPlayback();
  const firstSource = context.sources.at(-1);

  const second = playNotificationSound("flutter");
  await flushPlayback();

  assert.equal(firstEnded, 1);
  assert.equal(firstSource.stopped, true);
  assert.equal(
    fetchCalls.filter((url) => url.endsWith("flutter.mp3")).length,
    1,
  );
  assert.equal(context.decodeCalls, 1);
  assert.notEqual(context.sources.at(-1), firstSource);

  let secondEnded = 0;
  second.onEnded(() => {
    secondEnded += 1;
  });
  context.sources.at(-1).end();
  assert.equal(secondEnded, 1);
});

test("allows different notification cues to overlap", async () => {
  const context = FakeAudioContext.instances[0];
  playNotificationSound("dng");
  await flushPlayback();
  const firstSource = context.sources.at(-1);

  playNotificationSound("doo");
  await flushPlayback();

  assert.equal(firstSource.stopped, false);
  assert.equal(context.sources.at(-1).started, true);
  firstSource.end();
  context.sources.at(-1).end();
});

test("resumes a suspended context before starting a cue", async () => {
  const context = FakeAudioContext.instances[0];
  context.state = "suspended";

  playNotificationSound("ping");
  await flushPlayback();

  assert.equal(context.resumeCalls, 1);
  assert.equal(context.sources.at(-1).started, true);
});

test("a stopped loading cue never starts later", async () => {
  let releaseFetch;
  globalThis.fetch = async (url) => {
    fetchCalls.push(url);
    if (url.endsWith("boo.mp3")) {
      await new Promise((resolve) => {
        releaseFetch = resolve;
      });
    }
    return {
      ok: true,
      arrayBuffer: async () => new ArrayBuffer(4),
    };
  };

  const context = FakeAudioContext.instances[0];
  const sourceCount = context.sources.length;
  const playback = playNotificationSound("boo");
  playback.stop();
  releaseFetch();
  await flushPlayback();

  assert.equal(context.sources.length, sourceCount);
});

test("evicts failed loads so a later playback can retry", async () => {
  let attempts = 0;
  globalThis.fetch = async (url) => {
    fetchCalls.push(url);
    if (url.endsWith("oh-no.mp3") && attempts++ === 0) {
      return { ok: false, status: 503 };
    }
    return {
      ok: true,
      arrayBuffer: async () => new ArrayBuffer(4),
    };
  };

  const context = FakeAudioContext.instances[0];
  const sourceCount = context.sources.length;
  let failedEnded = 0;
  playNotificationSound("oh-no").onEnded(() => {
    failedEnded += 1;
  });
  await flushPlayback();

  assert.equal(failedEnded, 1);
  assert.equal(context.sources.length, sourceCount);

  playNotificationSound("oh-no");
  await flushPlayback();

  assert.equal(attempts, 2);
  assert.equal(context.sources.length, sourceCount + 1);
  context.sources.at(-1).end();
  globalThis.fetch = successfulFetch;
});
