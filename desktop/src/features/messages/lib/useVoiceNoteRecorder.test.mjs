import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

class FakeTrack {
  stopped = false;
  stop() {
    this.stopped = true;
  }
}

class FakeStream {
  track = new FakeTrack();
  getTracks() {
    return [this.track];
  }
}

class FakeRecorder extends dom.window.EventTarget {
  static isTypeSupported() {
    return true;
  }
  mimeType = "audio/webm";
  state = "inactive";
  start() {
    this.state = "recording";
  }
  stop() {
    if (this.state === "inactive") return;
    this.state = "inactive";
    this.dispatchEvent(
      new dom.window.MessageEvent("dataavailable", {
        data: new Blob([new Uint8Array([1])], { type: this.mimeType }),
      }),
    );
    this.dispatchEvent(new dom.window.Event("stop"));
  }
}

const decodeResolvers = [];
class FakeAudioContext {
  close() {
    return Promise.resolve();
  }
  createAnalyser() {
    return {
      fftSize: 0,
      smoothingTimeConstant: 0,
      getByteTimeDomainData() {},
    };
  }
  createMediaStreamSource() {
    return { connect() {} };
  }
  decodeAudioData() {
    return new Promise((resolve) => decodeResolvers.push(resolve));
  }
}

const streams = [];
const acquireFakeStream = async () => {
  const stream = new FakeStream();
  streams.push(stream);
  return stream;
};
let getUserMediaImpl = acquireFakeStream;
before(() => {
  Object.assign(globalThis, {
    AudioContext: FakeAudioContext,
    document: dom.window.document,
    DOMException: dom.window.DOMException,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    MediaRecorder: FakeRecorder,
    window: dom.window,
  });
  Object.defineProperty(dom.window.navigator, "mediaDevices", {
    configurable: true,
    value: {
      getUserMedia: (...args) => getUserMediaImpl(...args),
    },
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  dom.window.MediaRecorder = FakeRecorder;
  dom.window.AudioContext = FakeAudioContext;
});

after(() => dom.window.close());

test("permission acquisition is visible, cancellable, and releases a late stream", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useVoiceNoteRecorder } = await import("./useVoiceNoteRecorder.ts");
  const { result, unmount } = renderHook(() => useVoiceNoteRecorder());
  const stream = new FakeStream();
  let resolvePermission;
  getUserMediaImpl = () =>
    new Promise((resolve) => {
      resolvePermission = resolve;
    });

  try {
    let startPromise;
    act(() => {
      startPromise = result.current.start();
    });
    assert.equal(result.current.status, "requesting");

    act(() => result.current.cancel());
    assert.equal(result.current.status, "idle");

    await act(async () => {
      resolvePermission(stream);
      await startPromise;
    });
    assert.equal(stream.track.stopped, true);
    assert.equal(result.current.status, "idle");
  } finally {
    getUserMediaImpl = acquireFakeStream;
    unmount();
    cleanup();
  }
});

test("remains usable after Strict Mode replays the mount effect", async () => {
  const { StrictMode, createElement } = await import("react");
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useVoiceNoteRecorder } = await import("./useVoiceNoteRecorder.ts");
  const { result, unmount } = renderHook(() => useVoiceNoteRecorder(), {
    wrapper: ({ children }) => createElement(StrictMode, null, children),
  });

  try {
    await act(() => result.current.start());
    assert.equal(result.current.status, "recording");
    assert.equal(streams.at(-1).track.stopped, false);
  } finally {
    unmount();
    cleanup();
  }
});

test("a cancelled decode cannot stop or attach over a newer recording", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useVoiceNoteRecorder } = await import("./useVoiceNoteRecorder.ts");
  const { result, unmount } = renderHook(() => useVoiceNoteRecorder());

  try {
    await act(() => result.current.start());
    let firstFinish;
    await act(async () => {
      firstFinish = result.current.stop();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    assert.equal(decodeResolvers.length, 1);

    act(() => result.current.cancel());
    assert.equal(await firstFinish, null);
    await act(() => result.current.start());
    const secondTrack = streams.at(-1).track;

    await act(async () => {
      decodeResolvers.shift()({
        duration: 1,
        getChannelData: () => new Float32Array([0]),
        numberOfChannels: 1,
        sampleRate: 8_000,
      });
      await Promise.resolve();
    });

    assert.equal(secondTrack.stopped, false);
    assert.equal(result.current.status, "recording");
  } finally {
    unmount();
    cleanup();
  }
});
