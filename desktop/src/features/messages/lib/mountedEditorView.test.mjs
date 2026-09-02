import assert from "node:assert/strict";
import test from "node:test";

import { getMountedView } from "./mountedEditorView.ts";

// Mirrors tiptap v3's unmounted-view proxy: it stubs a few keys and throws for
// everything else, so reading `dom` is what blows up in production.
function unmountedViewProxy() {
  const stubs = { state: {}, composing: false, editable: true };
  return new Proxy(stubs, {
    get: (target, key) => {
      if (key in target) return Reflect.get(target, key);
      throw new Error(
        `[tiptap error]: The editor view is not available. Cannot access view['${String(key)}'].`,
      );
    },
  });
}

test("returns the view once it is mounted", () => {
  const view = { dom: { nodeType: 1 } };
  const editor = { isDestroyed: false, view };

  assert.equal(getMountedView(editor), view);
});

test("returns null instead of throwing while the view is unmounted", () => {
  const editor = { isDestroyed: false, view: unmountedViewProxy() };

  assert.equal(getMountedView(editor), null);
});

test("returns null for a destroyed editor without touching the view", () => {
  let viewReads = 0;
  const editor = {
    isDestroyed: true,
    get view() {
      viewReads += 1;
      throw new Error("view read on a destroyed editor");
    },
  };

  assert.equal(getMountedView(editor), null);
  assert.equal(viewReads, 0);
});

test("returns null when the view has no dom element", () => {
  const editor = { isDestroyed: false, view: { dom: null } };

  assert.equal(getMountedView(editor), null);
});
