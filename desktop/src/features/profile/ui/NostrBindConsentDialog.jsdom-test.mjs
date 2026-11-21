import assert from "node:assert/strict";
import test from "node:test";

const pending = {
  id: "cold-launch-request",
  payload: {
    challengeId: "550e8400-e29b-41d4-a716-446655440000",
    nonce: "z".repeat(43),
    verificationCode: "123456",
    audience: "buzz:nostr-identity",
    action: "bind_nostr_identity",
    protocol: "buzz-nostr-identity",
    version: "1",
    origin: "https://example.com",
    expiresAt: "2999-01-01T00:00:00Z",
    returnMode: "clipboard",
  },
};
let resolveTake;
const take = new Promise((resolve) => {
  resolveTake = resolve;
});
const calls = [];
const callbacks = new Map();
let nextCallback = 1;
const tauriMock = {
  invoke(command, args) {
    calls.push(command);
    if (command === "plugin:event|listen") return Promise.resolve(args.handler);
    if (command === "plugin:event|unlisten") return Promise.resolve();
    if (command === "take_pending_nostr_bind_deep_link") return take;
    if (command === "acknowledge_pending_nostr_bind_deep_link") {
      return Promise.resolve(true);
    }
    if (command === "get_identity") {
      return Promise.resolve({ pubkey: "a".repeat(64), display_name: "Test" });
    }
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback(callback) {
    const id = nextCallback++;
    callbacks.set(id, callback);
    return id;
  },
};
window.matchMedia = () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});
window.__TAURI_INTERNALS__ = tauriMock;
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
globalThis.__TAURI_INTERNALS__ = tauriMock;

const React = (await import("react")).default;
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { NostrBindConsentDialog } = await import("./NostrBindConsentDialog.tsx");

test("dialog unmount before native drain leaves the bind request unacknowledged", async () => {
  const root = createRoot(document.createElement("div"));
  await act(async () =>
    root.render(React.createElement(NostrBindConsentDialog)),
  );
  assert.ok(calls.includes("take_pending_nostr_bind_deep_link"));

  await act(async () => root.unmount());
  resolveTake(pending);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.ok(!calls.includes("acknowledge_pending_nostr_bind_deep_link"));
  assert.ok(!calls.includes("get_identity"));
});
