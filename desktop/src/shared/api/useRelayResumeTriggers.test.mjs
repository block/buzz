import assert from "node:assert/strict";
import { afterEach, describe, it } from "node:test";

import { installRelayResumeTriggers } from "./useRelayResumeTriggers.ts";

const originalDocument = globalThis.document;
const originalWindow = globalThis.window;

afterEach(() => {
  if (originalDocument === undefined) delete globalThis.document;
  else globalThis.document = originalDocument;
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
});

function installEventStub() {
  let visibilityState = "visible";
  const documentListeners = new Map();
  const windowListeners = new Map();
  const add = (listeners, type, listener) => {
    const values = listeners.get(type) ?? new Set();
    values.add(listener);
    listeners.set(type, values);
  };
  const remove = (listeners, type, listener) => {
    const values = listeners.get(type);
    values?.delete(listener);
    if (values?.size === 0) listeners.delete(type);
  };
  globalThis.document = {
    get visibilityState() {
      return visibilityState;
    },
    addEventListener: (type, listener) =>
      add(documentListeners, type, listener),
    removeEventListener: (type, listener) =>
      remove(documentListeners, type, listener),
  };
  globalThis.window = {
    addEventListener: (type, listener) => add(windowListeners, type, listener),
    removeEventListener: (type, listener) =>
      remove(windowListeners, type, listener),
  };
  return {
    documentListeners,
    windowListeners,
    emitDocument(type) {
      for (const listener of documentListeners.get(type) ?? []) listener();
    },
    emitWindow(type) {
      for (const listener of windowListeners.get(type) ?? []) listener();
    },
    setVisibility(value) {
      visibilityState = value;
    },
  };
}

describe("relay resume triggers", () => {
  it("defers and coalesces foreground signals", () => {
    const stub = installEventStub();
    const scheduled = [];
    let attempts = 0;
    const dispose = installRelayResumeTriggers(
      () => attempts++,
      (callback) => {
        const pending = { callback, cancelled: false };
        scheduled.push(pending);
        return () => {
          pending.cancelled = true;
        };
      },
    );

    stub.emitWindow("focus");
    stub.emitWindow("online");
    stub.emitDocument("visibilitychange");
    assert.equal(attempts, 0, "resume is not attempted in the event turn");
    assert.equal(scheduled.length, 1, "foreground burst is coalesced");

    scheduled[0].callback();
    assert.equal(attempts, 1);
    dispose();
    assert.equal(stub.documentListeners.size, 0);
    assert.equal(stub.windowListeners.size, 0);
  });

  it("cancels a pending resume when hidden", () => {
    const stub = installEventStub();
    let pending;
    let attempts = 0;
    installRelayResumeTriggers(
      () => attempts++,
      (callback) => {
        pending = {
          cancelled: false,
          run() {
            if (!this.cancelled) callback();
          },
        };
        return () => {
          pending.cancelled = true;
        };
      },
    );

    stub.emitWindow("focus");
    stub.setVisibility("hidden");
    stub.emitDocument("visibilitychange");
    assert.equal(pending.cancelled, true);
    pending.run();
    assert.equal(attempts, 0, "hidden state suppresses pending resume");
  });

  it("cancels a pending resume on dispose", () => {
    const stub = installEventStub();
    let pending;
    let attempts = 0;
    const dispose = installRelayResumeTriggers(
      () => attempts++,
      (callback) => {
        pending = {
          cancelled: false,
          run() {
            if (!this.cancelled) callback();
          },
        };
        return () => {
          pending.cancelled = true;
        };
      },
    );

    stub.emitWindow("focus");
    dispose();
    pending.run();
    assert.equal(attempts, 0);
    assert.equal(stub.documentListeners.size, 0);
    assert.equal(stub.windowListeners.size, 0);
  });
});
