import assert from "node:assert/strict";
import { afterEach, describe, it } from "node:test";

import {
  isDocumentVisible,
  subscribeDocumentVisibility,
} from "./useDocumentVisible.ts";

const originalDocument = globalThis.document;
const originalWindow = globalThis.window;

afterEach(() => {
  if (originalDocument === undefined) delete globalThis.document;
  else globalThis.document = originalDocument;
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
});

describe("document visibility", () => {
  it("defaults to visible when document is unavailable", () => {
    delete globalThis.document;
    assert.equal(isDocumentVisible(), true);
  });

  it("tracks visibility and focus changes and removes its listeners", () => {
    let visibilityState = "visible";
    let focused = true;
    const documentListeners = new Map();
    const windowListeners = new Map();
    globalThis.document = {
      get visibilityState() {
        return visibilityState;
      },
      hasFocus: () => focused,
      addEventListener(type, listener) {
        documentListeners.set(type, listener);
      },
      removeEventListener(type, listener) {
        if (documentListeners.get(type) === listener)
          documentListeners.delete(type);
      },
    };
    globalThis.window = {
      addEventListener(type, listener) {
        windowListeners.set(type, listener);
      },
      removeEventListener(type, listener) {
        if (windowListeners.get(type) === listener)
          windowListeners.delete(type);
      },
    };

    const observed = [];
    const unsubscribe = subscribeDocumentVisibility((visible) => {
      observed.push(visible);
    });

    focused = false;
    windowListeners.get("blur")();
    focused = true;
    windowListeners.get("focus")();
    visibilityState = "hidden";
    documentListeners.get("visibilitychange")();

    assert.deepEqual(observed, [false, true, false]);
    unsubscribe();
    assert.equal(documentListeners.size, 0);
    assert.equal(windowListeners.size, 0);
  });
});
