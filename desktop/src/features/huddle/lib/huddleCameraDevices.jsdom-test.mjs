import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import * as React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

const CHANNEL = "11111111-1111-4111-8111-111111111111";
let nativeCameraError = null;
let nextHandler = 0;
const callbacks = new Map();
const listeners = new Map();
let sentEvents = [];
let delayedListener = null;
function dispatch(event, payload) {
  for (const [id, listener] of [...listeners]) {
    if (listener.event === event)
      callbacks.get(listener.handler)?.({ event, id, payload });
  }
}
const tauri = {
  transformCallback: (callback) => {
    const id = ++nextHandler;
    callbacks.set(id, callback);
    return id;
  },
  invoke: async (command, args) => {
    if (command === "get_huddle_state")
      return {
        phase: "active",
        ephemeral_channel_id: CHANNEL,
        camera_share: null,
        camera_error: nativeCameraError,
      };
    if (command === "plugin:event|listen") {
      if (args.event === "huddle-camera-devices" && delayedListener) {
        const wait = delayedListener;
        delayedListener = null;
        await wait;
      }
      const id = ++nextHandler;
      listeners.set(id, args);
      return id;
    }
    if (command === "plugin:event|unlisten") {
      listeners.delete(args.eventId);
      return null;
    }
    if (command === "plugin:event|emit") {
      sentEvents.push(args);
      dispatch(args.event, args.payload);
      return null;
    }
    throw new Error(`Unexpected native operation: ${command}`);
  },
};
window.__TAURI_INTERNALS__ = tauri;
globalThis.__TAURI_INTERNALS__ = tauri;
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
const { relayClient } = await import("@/shared/api/relayClient");
relayClient.subscribeLive = async () => {};
relayClient.subscribeToConnectionState = () => () => {};
relayClient.subscribeToChannelAccessRevocations = () => () => {};
const { HuddleCameraProvider, useHuddleCamera } = await import(
  "../HuddleCameraContext.tsx"
);

const camera = (deviceId = "usb") => ({
  kind: "videoinput",
  deviceId,
  label: `Camera ${deviceId}`,
});
const previous = Object.getOwnPropertyDescriptor(navigator, "mediaDevices");
let root;
let container;
let latest;
let companion;
let devices;
let calls;
let announcements;

function Probe({ peer = false }) {
  const value = useHuddleCamera();
  if (peer) companion = value;
  else latest = value;
  return null;
}

async function render(channelId = CHANNEL) {
  await act(async () => {
    root.render(
      React.createElement(
        HuddleCameraProvider,
        { channelId, ownsCapture: true },
        React.createElement(Probe),
      ),
    );
  });
}

beforeEach(() => {
  nativeCameraError = null;
  callbacks.clear();
  listeners.clear();
  companion = null;
  sentEvents = [];
  delayedListener = null;
  calls = 0;
  announcements = new Set();
  devices = new window.EventTarget();
  const source = devices;
  devices.enumerateDevices = async () => {
    calls++;
    // Keep a regressed feedback loop finite so the regression fails promptly.
    if (calls > 3) throw new Error("Enumeration feedback loop");
    const timer = setTimeout(() => {
      announcements.delete(timer);
      source.dispatchEvent(new Event("devicechange"));
    }, 0);
    announcements.add(timer);
    return [camera()];
  };
  Object.defineProperty(navigator, "mediaDevices", {
    configurable: true,
    value: devices,
  });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  for (const timer of announcements) clearTimeout(timer);
  container.remove();
  if (previous) Object.defineProperty(navigator, "mediaDevices", previous);
  else delete navigator.mediaDevices;
});

test("the production camera provider never enumerates while idle or from devicechange storms", async () => {
  await render(null);
  assert.equal(calls, 0);
  await render();
  await act(async () => new Promise((resolve) => setTimeout(resolve, 30)));
  assert.equal(calls, 1, "huddle entry permits one probe");
  assert.deepEqual(latest.cameras, [{ deviceId: "usb", label: "Camera usb" }]);
  await act(async () => {
    for (let index = 0; index < 20; index++)
      devices.dispatchEvent(new Event("devicechange"));
  });
  assert.equal(
    calls,
    1,
    "WebKit re-announcements must not feed another enumeration",
  );
  await act(async () => latest.refresh());
  assert.equal(calls, 2, "explicit refresh permits one additional probe");
});

test("overlapping camera refreshes share one probe and preserve an unchanged list", async () => {
  await render();
  const initial = latest.cameras;
  let resolve;
  devices.enumerateDevices = () => {
    calls++;
    return new Promise((done) => {
      resolve = done;
    });
  };
  let first;
  let second;
  await act(async () => {
    first = latest.refresh();
    second = latest.refresh();
  });
  assert.equal(calls, 2);
  assert.equal(latest.checking, true);
  await act(async () => {
    resolve([camera()]);
    await Promise.all([first, second]);
  });
  assert.equal(latest.cameras, initial);
  assert.equal(latest.checking, false);
});

test("a late camera probe cannot repopulate a room that was left", async () => {
  let resolve;
  devices.enumerateDevices = () =>
    new Promise((done) => {
      resolve = done;
    });
  await render();
  assert.equal(latest.checking, true);
  await render(null);
  await act(async () => resolve([camera("late")]));
  assert.deepEqual(latest.cameras, []);
  assert.equal(latest.checking, false);
});

test("camera enumeration failures remain visible and explicit retry clears only that error", async () => {
  await render();
  const initial = latest.cameras;
  devices.enumerateDevices = async () => {
    throw new Error("Device monitor failed");
  };
  await act(async () => latest.refresh());
  assert.match(latest.error, /Device monitor failed/);
  assert.equal(latest.cameras, initial);
  assert.equal(latest.checking, false);
  devices.enumerateDevices = async () => [camera("replacement")];
  await act(async () => latest.refresh());
  assert.equal(latest.error, null);
  assert.deepEqual(latest.cameras, [
    { deviceId: "replacement", label: "Camera replacement" },
  ]);
});

test("refreshing device discovery does not erase a native camera failure", async () => {
  nativeCameraError = "Camera disconnected";
  await render();
  await act(async () => latest.refresh());
  assert.equal(latest.error, "Camera disconnected");
});

async function renderCompanion(withOwner = true) {
  await act(async () => {
    root.render(
      React.createElement(
        React.Fragment,
        null,
        withOwner
          ? React.createElement(
              HuddleCameraProvider,
              { channelId: CHANNEL, ownsCapture: true },
              React.createElement(Probe),
            )
          : null,
        React.createElement(
          HuddleCameraProvider,
          { channelId: null, ownsCapture: false },
          React.createElement(Probe, { peer: true }),
        ),
      ),
    );
  });
}

test("a companion never enumerates its own permission-redacted devices", async () => {
  await renderCompanion(false);
  assert.equal(calls, 0);
  assert.equal(companion.checking, true);
});

test("the production companion receives owner device IDs and delegates refresh", async () => {
  await renderCompanion();
  assert.deepEqual(companion.cameras, [
    { deviceId: "usb", label: "Camera usb" },
  ]);
  const before = calls;
  devices.enumerateDevices = async () => {
    calls++;
    return [camera("monitor")];
  };
  await act(async () => companion.refresh());
  assert.equal(calls, before + 1, "only the capture owner probes the hardware");
  assert.deepEqual(companion.cameras, latest.cameras);
  assert.deepEqual(companion.cameras, [
    { deviceId: "monitor", label: "Camera monitor" },
  ]);
});

test("a companion request sent before the owner subscribes is retried on owner readiness", async () => {
  let ready;
  delayedListener = new Promise((resolve) => {
    ready = resolve;
  });
  await renderCompanion();
  assert.deepEqual(companion.cameras, []);
  await act(async () => ready());
  assert.deepEqual(companion.cameras, [
    { deviceId: "usb", label: "Camera usb" },
  ]);
  assert.equal(companion.checking, false);
});

test("owner discovery failures reach the companion and an explicit retry recovers", async () => {
  await renderCompanion();
  const previous = companion.cameras;
  devices.enumerateDevices = async () => {
    throw new Error("Monitor temporarily unavailable");
  };
  await act(async () => companion.refresh());
  assert.match(companion.error, /Monitor temporarily unavailable/);
  assert.equal(companion.cameras, previous);
  assert.equal(companion.checking, false);
  devices.enumerateDevices = async () => [camera("monitor")];
  await act(async () => companion.refresh());
  assert.equal(companion.error, null);
  assert.equal(companion.cameras[0].deviceId, "monitor");
});

test("unrelated and retired huddle replies cannot replace companion devices", async () => {
  await renderCompanion();
  let resolve;
  devices.enumerateDevices = () =>
    new Promise((done) => {
      resolve = done;
    });
  let refresh;
  await act(async () => {
    refresh = companion.refresh();
  });
  const requestId = sentEvents.findLast(
    (event) => event.payload?.type === "request",
  ).payload.requestId;
  await act(async () => {
    dispatch("huddle-camera-devices", {
      channel: "another-room",
      type: "result",
      requestId,
      result: { cameras: [{ deviceId: "foreign", label: "Foreign camera" }] },
    });
    dispatch("huddle-camera-devices", {
      channel: CHANNEL,
      type: "result",
      requestId: "retired",
      result: { cameras: [{ deviceId: "retired", label: "Retired camera" }] },
    });
  });
  assert.equal(companion.cameras[0].deviceId, "usb");
  assert.equal(companion.checking, true);
  await act(async () => {
    dispatch("huddle-state-changed", {
      phase: "idle",
      ephemeral_channel_id: null,
      camera_share: null,
    });
  });
  assert.deepEqual(companion.cameras, []);
  await act(async () => {
    resolve([camera("late")]);
    await refresh;
  });
  assert.deepEqual(companion.cameras, []);
  assert.equal(companion.checking, false);
});

test("a missing owner times out with a retryable error", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  try {
    await renderCompanion(false);
    await act(async () => t.mock.timers.tick(5000));
    assert.equal(companion.checking, false);
    assert.match(companion.error, /capture window did not respond/);
  } finally {
    t.mock.timers.reset();
  }
});
