import assert from "node:assert/strict";
import { after, beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";
import { useImplicitAgentMentionProvenance } from "./useImplicitAgentMentionProvenance.ts";
import { useDraftPersistLifecycle } from "./useDraftPersistSnapshot.ts";
import {
  clearAllDrafts,
  initDraftStore,
  loadDraftEntry,
  persistDraftEntry,
} from "../lib/useDrafts.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  localStorage: dom.window.localStorage,
  IS_REACT_ACT_ENVIRONMENT: true,
});
const { act, cleanup, renderHook } = await import("@testing-library/react");
beforeEach(() => {
  cleanup();
  clearAllDrafts();
  initDraftStore("author", "wss://provenance.example");
});
after(() => {
  cleanup();
  dom.window.close();
});
const A = "a".repeat(64);
const B = "b".repeat(64);
const fragment = (pubkey, label) => ({ pubkey, prefix: `@${label} ` });

function mount() {
  let text = "";
  const hook = renderHook(
    ({ key }) => {
      const provenance = useImplicitAgentMentionProvenance(key);
      const lifecycle = useDraftPersistLifecycle({
        effectiveDraftKey: key,
        channelId: key,
        loadDraft: loadDraftEntry,
        persistDraft: persistDraftEntry,
        getMentionRefs: () => [],
        restoreMentionRefs: () => {},
        livePendingImeta: [],
        setPendingImeta: () => {},
        setContent: (value) => {
          text = value;
        },
        clearContent: () => {
          text = "";
        },
        setSpoileredAttachmentUrls: () => {},
        spoileredAttachmentUrlsRef: { current: new Set() },
        syncComposerContentFromEditor: () => text,
        getImplicitAgentMentionPrefix: provenance.getPrefix,
      });
      return { provenance, lifecycle };
    },
    { initialProps: { key: "channel" } },
  );
  return {
    ...hook,
    text: () => text,
    visit: (key) => hook.rerender({ key }),
    // Same boundary as picker insertion: record exact fragments before updating text.
    generate: (fragments) =>
      act(() => {
        hook.result.current.lifecycle.runComposerUpdate(() => {
          hook.result.current.provenance.add(fragments);
          text = fragments.map(({ prefix }) => prefix).join("");
          hook.result.current.lifecycle.trackAuthoredContent(text);
        });
      }),
    author: (value) =>
      act(() => {
        text = value;
        hook.result.current.lifecycle.trackAuthoredContent(value);
      }),
  };
}

for (const [oldLabel, newLabel] of [
  [`Scout (${B})`, "Scout"],
  ["Scout", "Scout Jones"],
  ["Scout Jones", `Scout Jones (${B})`],
]) {
  for (const body of ["", "authored body", `@${newLabel} authored duplicate`]) {
    test(`${oldLabel} → ${newLabel}: persists only ${JSON.stringify(body)}`, () => {
      const h = mount();
      h.generate([fragment(B, oldLabel)]);
      const revision = h.result.current.lifecycle.getComposerRevision();
      h.generate([fragment(B, newLabel)]);
      assert.equal(h.result.current.lifecycle.getComposerRevision(), revision);
      if (body) h.author(`@${newLabel} ${body}`);
      h.visit("other");
      assert.equal(loadDraftEntry("channel")?.content ?? "", body);
      h.visit("channel");
      assert.equal(h.text(), body);
      assert.equal(h.result.current.provenance.getPrefix(), `@${newLabel} `);
    });
  }
}

test("new insertion order and fragments replace known keys, preserving other keys", () => {
  const h = mount();
  const p = () => h.result.current.provenance;
  act(() => p().add([fragment(A, "Scout"), fragment(B, `Scout (${B})`)]));
  act(() => p().add([fragment(B, "Scout Jones")]));
  assert.equal(p().getPrefix(), "@Scout Jones @Scout ");
  act(() => p().add([fragment(A, "Scout"), fragment(B, "Scout Jones")]));
  assert.equal(p().getPrefix(), "@Scout @Scout Jones ");
  act(() => p().remove(A));
  assert.equal(p().getPrefix(), "@Scout Jones ");
  act(() => p().remove(B));
  assert.equal(p().getPrefix(), "");
  h.author("@Scout Jones authored after removal");
  h.visit("other");
  assert.equal(
    loadDraftEntry("channel").content,
    "@Scout Jones authored after removal",
  );
});

test("changed generated label cannot override authored empty authority", () => {
  const h = mount();
  h.generate([fragment(B, `Scout (${B})`)]);
  h.author("");
  const revision = h.result.current.lifecycle.getComposerRevision();
  h.generate([fragment(B, "Scout")]);
  assert.equal(h.result.current.lifecycle.getComposerRevision(), revision);
  h.visit("other");
  assert.equal(loadDraftEntry("channel"), undefined);
  h.visit("channel");
  assert.equal(h.text(), "");
});

test("changed labels are draft scoped and absent keys do not capture provenance", () => {
  const h = mount();
  h.generate([fragment(B, `Scout (${B})`)]);
  h.visit("other");
  h.generate([fragment(B, "Scout")]);
  h.visit("channel");
  assert.equal(h.result.current.provenance.getPrefix(), `@Scout (${B}) `);
  h.visit(null);
  h.generate([fragment(B, "No Draft")]);
  assert.equal(h.result.current.provenance.getPrefix(), "");
});
