import assert from "node:assert/strict";
import { after, beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";
import { useAddressedAgentMentionRestore } from "./useAddressedAgentMentionRestore.ts";
import { useDraftPersistLifecycle } from "./useDraftPersistSnapshot.ts";
import {
  claimDraftSend,
  clearAllDrafts,
  deleteDraftEntry,
  getDraftAuthority,
  initDraftStore,
  loadDraftEntry,
  persistDraftEntry,
  recordDraftAuthoredContent,
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
after(() => {
  cleanup();
  dom.window.close();
});
let frames;
let nextFrame = 0;
beforeEach(() => {
  cleanup();
  clearAllDrafts();
  initDraftStore("author", "wss://restore.example");
  frames = new Map();
  // Keep cancelled callbacks invocable: authority must reject an already dequeued frame too.
  globalThis.requestAnimationFrame = (cb) => {
    frames.set(++nextFrame, cb);
    return nextFrame;
  };
  globalThis.cancelAnimationFrame = () => {};
});
const A = "a".repeat(64);
const B = "b".repeat(64);
function mount() {
  let text = "";
  const writes = [];
  let lifecycle;
  const hook = renderHook(
    ({ key, channelId, enabled }) => {
      lifecycle = useDraftPersistLifecycle({
        effectiveDraftKey: key,
        channelId,
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
      });
      const restore = useAddressedAgentMentionRestore({
        audiencePubkeys: [A, B],
        channelId,
        enabled,
        getComposerRevision: lifecycle.getComposerRevision,
        runComposerUpdate: lifecycle.runComposerUpdate,
      });
      restore.restoreAddressedAgentMentionsRef.current = (keys, allowed) => {
        writes.push([keys, allowed]);
        text = "@Agent Ada ";
        lifecycle.trackAuthoredContent(text);
        return text;
      };
      return restore;
    },
    { initialProps: { key: "A", channelId: "channel", enabled: true } },
  );
  return {
    ...hook,
    writes,
    author: (value) =>
      act(() => {
        text = value;
        lifecycle.trackAuthoredContent(value);
      }),
    schedule: (keys = [A]) => {
      act(() => hook.result.current.onAddressedAgentsSendSucceeded(keys, keys));
      return nextFrame;
    },
    release: (id) => act(() => frames.get(id)(0)),
    visit: (key, enabled = true) =>
      hook.rerender({ key, channelId: "channel", enabled }),
  };
}

for (const [name, supersede] of [
  ["authored content", (h) => h.author("follow up")],
  ["authored empty", (h) => h.author("")],
  ["other visit authors empty", () => recordDraftAuthoredContent("A", "")],
  ["explicit deletion of absent value", () => deleteDraftEntry("A")],
  ["new send", () => claimDraftSend("A")],
  [
    "scope reset round trip",
    () => {
      initDraftStore("other");
      initDraftStore("author", "wss://restore.example");
    },
  ],
  [
    "same-channel draft visit round trip",
    (h) => {
      h.visit("B");
      h.visit("A");
    },
  ],
  [
    "disable then enable",
    (h) => {
      h.visit("A", false);
      h.visit("A");
    },
  ],
  ["unmount", (h) => h.unmount()],
]) {
  test(`delayed automatic restore loses authority after ${name}`, () => {
    const h = mount();
    const frame = h.schedule();
    supersede(h);
    h.release(frame);
    assert.deepEqual(h.writes, []);
  });
}

test("no author restores exact captured keys without manufacturing authored intent", () => {
  const h = mount();
  const authority = getDraftAuthority("A");
  const revision = authority.revision;
  const frame = h.schedule([B]);
  h.release(frame);
  assert.deepEqual(h.writes, [[[B], [B]]]);
  assert.equal(authority.revision, revision);
  assert.equal(authority.authoredRevision, 0);
});
test("restore first then author, and unrelated key authoring, preserve authority", () => {
  const h = mount();
  const frame = h.schedule();
  recordDraftAuthoredContent("B", "other");
  h.release(frame);
  h.author("");
  assert.equal(h.writes.length, 1);
  assert.equal(getDraftAuthority("A").emptyContentIsAuthoritative, true);
});
test("a replaced frame cannot restore old recipients or consume the current frame", () => {
  const h = mount();
  const old = h.schedule([A]);
  const current = h.schedule([B]);
  h.release(old);
  assert.deepEqual(h.writes, []);
  h.release(current);
  assert.deepEqual(h.writes, [[[B], [B]]]);
});
test("synchronous automatic clear restoration is programmatic too", () => {
  const h = mount();
  const revision = getDraftAuthority("A").revision;
  act(() => h.result.current.onAddressedAgentsComposerCleared([A]));
  assert.equal(getDraftAuthority("A").revision, revision);
  assert.deepEqual(h.writes, [[[A], undefined]]);
});
