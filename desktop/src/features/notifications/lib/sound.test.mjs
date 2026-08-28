import assert from "node:assert/strict";
import test from "node:test";

import {
  KIND_APPROVAL_REQUEST,
  KIND_STREAM_MESSAGE_V2,
  KIND_JOB_ACCEPTED,
} from "../../../shared/constants/kinds.ts";
import {
  DEFAULT_SLOT_ALERTS_ENABLED,
  playNotificationSound,
  resetNotificationSoundCache,
  shouldPlayNotificationSound,
  slotForFeedKind,
  SOUND_SLOTS,
} from "./sound.ts";

test("all-messages alerts are first and on by default", () => {
  assert.equal(SOUND_SLOTS[0], "all_messages");
  assert.equal(DEFAULT_SLOT_ALERTS_ENABLED.all_messages, true);
});

test("routes each feed category to its own sound slot", () => {
  assert.equal(slotForFeedKind(KIND_STREAM_MESSAGE_V2, "mention"), "mention");
  assert.equal(
    slotForFeedKind(KIND_APPROVAL_REQUEST, "needs_action"),
    "needs_action",
  );
  assert.equal(
    slotForFeedKind(KIND_STREAM_MESSAGE_V2, "activity"),
    "needs_action",
  );
  assert.equal(
    slotForFeedKind(KIND_STREAM_MESSAGE_V2, "agent_activity"),
    "needs_action",
  );
});

test("agent job kinds pick their slot for non-mention categories", () => {
  assert.equal(
    slotForFeedKind(KIND_JOB_ACCEPTED, "agent_activity"),
    "job_accepted",
  );
});

test("a mention outranks the agent job kind that carried it", () => {
  assert.equal(slotForFeedKind(KIND_JOB_ACCEPTED, "mention"), "mention");
});

test("unknown category falls back to needs_action and warns", () => {
  const warnings = [];
  const originalWarn = console.warn;
  console.warn = (...args) => warnings.push(args.join(" "));
  try {
    // The backend once emitted the plural section name here; the fallback
    // keeps the user alerted while the warning keeps the drift visible.
    assert.equal(
      slotForFeedKind(KIND_STREAM_MESSAGE_V2, "mentions"),
      "needs_action",
    );
  } finally {
    console.warn = originalWarn;
  }
  assert.equal(warnings.length, 1);
  assert.match(warnings[0], /unknown feed item category "mentions"/);
});

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

test("plays notification sounds from a blob URL, not the custom-scheme path", async (t) => {
  const originalAudio = globalThis.Audio;
  const originalFetch = globalThis.fetch;
  const originalCreateObjectURL = URL.createObjectURL;
  const fetches = [];

  class FakeAudio {
    constructor(src) {
      this.src = src;
      this.currentTime = 99;
      this.playCalls = 0;
    }

    play() {
      this.playCalls += 1;
      return Promise.resolve();
    }
  }

  globalThis.Audio = FakeAudio;
  globalThis.fetch = async (url) => {
    fetches.push(url);
    return {
      ok: true,
      arrayBuffer: async () => new Uint8Array([1, 2, 3]).buffer,
    };
  };
  URL.createObjectURL = (blob) => {
    assert.equal(blob.type, "audio/mpeg");
    return "blob:notification-sound";
  };
  t.after(() => {
    resetNotificationSoundCache();
    globalThis.Audio = originalAudio;
    globalThis.fetch = originalFetch;
    URL.createObjectURL = originalCreateObjectURL;
  });

  const first = await playNotificationSound("bong");
  const second = await playNotificationSound("bong");

  assert.equal(fetches.length, 1);
  assert.equal(fetches[0], "/sounds/bong.mp3");
  assert.ok(first);
  assert.equal(first.src, "blob:notification-sound");
  assert.equal(first.currentTime, 0);
  assert.equal(first.playCalls, 2);
  assert.equal(second, first);
});

test("autoplay rejection falls back to AudioContext playback", async (t) => {
  const originalAudio = globalThis.Audio;
  const originalAudioContext = globalThis.AudioContext;
  const originalFetch = globalThis.fetch;
  const originalCreateObjectURL = URL.createObjectURL;
  let started = 0;

  class FakeAudio {
    constructor(src) {
      this.src = src;
      this.currentTime = 0;
    }

    play() {
      return Promise.reject(
        Object.assign(new Error("play blocked"), { name: "NotAllowedError" }),
      );
    }
  }

  class FakeAudioContext {
    state = "running";
    destination = {};
    resume() {
      return Promise.resolve();
    }
    decodeAudioData() {
      return Promise.resolve({ duration: 0.2 });
    }
    createBufferSource() {
      return {
        buffer: null,
        connect() {},
        start() {
          started += 1;
        },
      };
    }
  }

  globalThis.Audio = FakeAudio;
  globalThis.AudioContext = FakeAudioContext;
  globalThis.fetch = async () => ({
    ok: true,
    arrayBuffer: async () => new Uint8Array([1, 2, 3]).buffer,
  });
  URL.createObjectURL = () => "blob:notification-sound";
  t.after(() => {
    resetNotificationSoundCache();
    globalThis.Audio = originalAudio;
    globalThis.AudioContext = originalAudioContext;
    globalThis.fetch = originalFetch;
    URL.createObjectURL = originalCreateObjectURL;
  });

  const audio = await playNotificationSound("doop");
  assert.ok(audio);
  assert.equal(started, 1);
});

test("live alerts prefer AudioContext even when HTMLAudio.play resolves", async (t) => {
  const originalAudio = globalThis.Audio;
  const originalAudioContext = globalThis.AudioContext;
  const originalFetch = globalThis.fetch;
  const originalCreateObjectURL = URL.createObjectURL;
  let started = 0;
  let elementPlayCalls = 0;

  class FakeAudio {
    constructor(src) {
      this.src = src;
      this.currentTime = 0;
    }

    play() {
      elementPlayCalls += 1;
      return Promise.resolve();
    }
  }

  class FakeAudioContext {
    state = "running";
    destination = {};
    resume() {
      return Promise.resolve();
    }
    decodeAudioData() {
      return Promise.resolve({ duration: 0.2 });
    }
    createBufferSource() {
      return {
        buffer: null,
        connect() {},
        start() {
          started += 1;
        },
      };
    }
  }

  globalThis.Audio = FakeAudio;
  globalThis.AudioContext = FakeAudioContext;
  globalThis.fetch = async () => ({
    ok: true,
    arrayBuffer: async () => new Uint8Array([1, 2, 3]).buffer,
  });
  URL.createObjectURL = () => "blob:notification-sound";
  t.after(() => {
    resetNotificationSoundCache();
    globalThis.Audio = originalAudio;
    globalThis.AudioContext = originalAudioContext;
    globalThis.fetch = originalFetch;
    URL.createObjectURL = originalCreateObjectURL;
  });

  await playNotificationSound("ping");
  assert.equal(started, 1);
  assert.equal(elementPlayCalls, 0);

  await playNotificationSound("ping", { preview: true });
  assert.equal(elementPlayCalls, 1);
});

test("a failed sound fetch does not stick in the cache", async (t) => {
  const originalAudio = globalThis.Audio;
  const originalFetch = globalThis.fetch;
  const originalWarn = console.warn;
  const warnings = [];
  let calls = 0;

  globalThis.Audio = class {
    play() {
      return Promise.resolve();
    }
  };
  globalThis.fetch = async () => {
    calls += 1;
    return {
      ok: false,
      status: 404,
      arrayBuffer: async () => new ArrayBuffer(0),
    };
  };
  console.warn = (...args) => warnings.push(args.join(" "));
  t.after(() => {
    resetNotificationSoundCache();
    globalThis.Audio = originalAudio;
    globalThis.fetch = originalFetch;
    console.warn = originalWarn;
  });

  assert.equal(await playNotificationSound("ping"), null);
  assert.equal(await playNotificationSound("ping"), null);
  assert.equal(calls, 2);
  assert.equal(warnings.length, 2);
  assert.match(warnings[0], /sound play failed/);
});
