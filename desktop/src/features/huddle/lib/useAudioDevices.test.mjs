import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/**
 * Regression environment for the Linux media device-enumeration loop
 * (docs/linux-media-device-enumeration-loop.md).
 *
 * The fake `mediaDevices` reproduces the measured WebKitGTK pathology: every
 * `enumerateDevices()` call starts fresh GStreamer device-monitor machinery,
 * and each monitor start re-announces the existing devices as `devicechange`
 * shortly after resolving. Under the old mount-enumerate + `devicechange`
 * listener this fed itself forever, leaking file descriptors per cycle in the
 * real web process until it died. The demand-driven hook must converge to a
 * bounded number of enumerations driven only by explicit triggers.
 */

const enumerateCalls = [];
let deviceChangeListeners = new Set();
let enumerateImpl = async () => [];

function makeDevice(deviceId, label) {
  return { deviceId, kind: "audioinput", label, toJSON() {} };
}

function dispatchDeviceChange() {
  for (const listener of [...deviceChangeListeners]) {
    listener(new dom.window.Event("devicechange"));
  }
}

/**
 * Pathological enumerate: resolves with `devices`, then schedules the
 * WebKitGTK-style re-announcement. Macrotask-delayed so a reintroduced
 * event-driven listener fails the assertions quickly instead of starving the
 * microtask queue.
 */
function installStormingEnumerate(devices) {
  enumerateImpl = async () => {
    enumerateCalls.push("enumerate");
    setTimeout(dispatchDeviceChange, 0);
    return devices;
  };
}

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  Object.defineProperty(dom.window.navigator, "mediaDevices", {
    configurable: true,
    value: {
      addEventListener(type, listener) {
        if (type === "devicechange") deviceChangeListeners.add(listener);
      },
      removeEventListener(_type, listener) {
        deviceChangeListeners.delete(listener);
      },
      enumerateDevices: (...args) => enumerateImpl(...args),
    },
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
});

beforeEach(() => {
  enumerateCalls.length = 0;
  deviceChangeListeners = new Set();
});

after(() => dom.window.close());

async function renderAudioDevicesHook() {
  const { renderHook } = await import("@testing-library/react");
  const { useAudioDevices } = await import("./useAudioDevices.ts");
  const workletRef = { current: null };
  const renderCounts = { value: 0 };
  const rendered = renderHook(() => {
    renderCounts.value += 1;
    return useAudioDevices(workletRef);
  });
  return { ...rendered, renderCounts };
}

test("mount performs zero enumerations and simulated devicechange storms trigger none", async () => {
  const { act, cleanup } = await import("@testing-library/react");
  installStormingEnumerate([makeDevice("mic-a", "Built-in Microphone")]);

  const { result, unmount } = await renderAudioDevicesHook();
  try {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    assert.equal(enumerateCalls.length, 0, "mount must not enumerate devices");
    assert.deepEqual(result.current.audioDevices, []);

    // Reintroducing either the mount enumeration or a `devicechange`
    // listener makes the count diverge instead of staying at zero.
    await act(async () => {
      for (let i = 0; i < 50; i += 1) {
        dispatchDeviceChange();
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
    });
    assert.equal(
      enumerateCalls.length,
      0,
      "devicechange re-announcements must not trigger enumeration",
    );
  } finally {
    unmount();
    cleanup();
  }
});

test("overlapping refreshes serialize into a single enumeration", async () => {
  const { act, cleanup } = await import("@testing-library/react");
  let releaseFirst;
  enumerateImpl = () => {
    enumerateCalls.push("enumerate");
    if (enumerateCalls.length === 1) {
      return new Promise((resolve) => {
        releaseFirst = () =>
          resolve([makeDevice("mic-a", "Built-in Microphone")]);
      });
    }
    return Promise.resolve([makeDevice("mic-a", "Built-in Microphone")]);
  };

  const { result, unmount } = await renderAudioDevicesHook();
  try {
    let first;
    let second;
    await act(async () => {
      first = result.current.refreshAudioDevices();
      second = result.current.refreshAudioDevices();
      releaseFirst();
      await Promise.all([first, second]);
    });

    assert.equal(
      enumerateCalls.length,
      1,
      "a call made while a refresh is in flight must await it, not enumerate again",
    );
  } finally {
    unmount();
    cleanup();
  }
});

test("identical device lists do not update state; changed lists do", async () => {
  const { act, cleanup } = await import("@testing-library/react");
  installStormingEnumerate([makeDevice("mic-a", "Built-in Microphone")]);

  const { result, unmount, renderCounts } = await renderAudioDevicesHook();
  try {
    await act(async () => {
      await result.current.refreshAudioDevices();
    });
    const rendersAfterFirstRefresh = renderCounts.value;
    assert.deepEqual(result.current.audioDevices, [
      { deviceId: "mic-a", label: "Built-in Microphone" },
    ]);

    await act(async () => {
      await result.current.refreshAudioDevices();
    });
    assert.equal(
      renderCounts.value,
      rendersAfterFirstRefresh,
      "an identical device list must not trigger a re-render",
    );

    enumerateImpl = async () => {
      enumerateCalls.push("enumerate");
      return [makeDevice("mic-b", "USB Microphone")];
    };
    await act(async () => {
      await result.current.refreshAudioDevices();
    });
    assert.equal(renderCounts.value, rendersAfterFirstRefresh + 1);
    assert.deepEqual(result.current.audioDevices, [
      { deviceId: "mic-b", label: "USB Microphone" },
    ]);
  } finally {
    unmount();
    cleanup();
  }
});

test("a failed enumeration keeps the previous list and does not reject", async () => {
  const { act, cleanup } = await import("@testing-library/react");
  enumerateImpl = async () => {
    enumerateCalls.push("enumerate");
    return [makeDevice("mic-a", "Built-in Microphone")];
  };

  const { result, unmount } = await renderAudioDevicesHook();
  try {
    await act(async () => {
      await result.current.refreshAudioDevices();
    });

    enumerateImpl = async () => {
      enumerateCalls.push("enumerate");
      throw new Error("enumeration failed");
    };
    await act(async () => {
      await result.current.refreshAudioDevices();
    });

    assert.deepEqual(
      result.current.audioDevices,
      [{ deviceId: "mic-a", label: "Built-in Microphone" }],
      "a failed refresh must keep the last known list",
    );
  } finally {
    unmount();
    cleanup();
  }
});
