import assert from "node:assert/strict";
import test from "node:test";

const listeners = new Map();
const values = new Map();
globalThis.window = {
  localStorage: {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  },
  addEventListener: (name, listener) => listeners.set(name, listener),
  removeEventListener: (name) => listeners.delete(name),
  dispatchEvent: () => true,
};

const {
  getProviderUsagePreference,
  resolveProviderUsagePreference,
  setProviderUsagePreference,
} = await import("./providerUsagePreference.ts");

test("provider preference defaults to Auto and persists supported values", () => {
  assert.equal(getProviderUsagePreference(), "auto");
  setProviderUsagePreference("codex");
  assert.equal(getProviderUsagePreference(), "codex");
});

test("provider preference rejects malformed storage", () => {
  values.set("buzz-provider-usage-preference", "secret-provider");
  assert.equal(getProviderUsagePreference(), "auto");
});

test("provider preference tolerates unavailable storage", () => {
  const original = globalThis.window.localStorage.getItem;
  globalThis.window.localStorage.getItem = () => {
    throw new Error("unavailable");
  };
  assert.equal(getProviderUsagePreference(), "auto");
  globalThis.window.localStorage.getItem = original;
});

test("Auto resolves to the supported Codex adapter", () => {
  assert.equal(resolveProviderUsagePreference("auto"), "codex");
  assert.equal(resolveProviderUsagePreference("grok"), "grok");
  assert.equal(
    resolveProviderUsagePreference("auto", [
      {
        id: "claude",
        name: "Claude",
        availability: "available",
        detail: "Future supported adapter",
      },
    ]),
    "claude",
  );
});
