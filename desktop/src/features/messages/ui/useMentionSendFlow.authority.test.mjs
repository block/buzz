import assert from "node:assert/strict";
import { test } from "node:test";
import {
  setup,
  deferred,
  KEY,
  TEXT,
} from "./useMentionSendFlow.test-support.mjs";

async function ordinary(s, gate) {
  s.options.mentions.memberPubkeys = new Set([KEY]);
  s.control.publish = gate;
  s.rerender();
  let promise;
  await s.act(async () => {
    promise = s.result.current.sendMessageWithMentionFlow({
      capturedChannelId: "general",
      pendingImeta: [],
      trimmed: TEXT,
      recoveryDraftKey: "thread:a",
      sentDraftKey: "thread:a",
    });
  });
  return { promise };
}

// Bound the transition product, not just one reporter sequence. Same snapshot
// authorship must supersede cleanup too; equality of content/refs is not intent.
for (const outcome of ["failure", "success"]) {
  for (const intent of [
    "untouched",
    "clear",
    "text",
    "same-text",
    "new-refs",
  ]) {
    for (const exit of ["B", "unmount"]) {
      test(`ordinary cross-visit ${outcome}, newer ${intent}, exit ${exit}`, async () => {
        const s = await setup({ lifecycle: true });
        s.dismiss();
        const gate = deferred();
        const { promise } = await ordinary(s, gate);
        assert.equal(s.events("publish").length, 1);
        assert.equal(s.options.contentRef.current, "");
        s.navigate("thread:b");
        s.edit("B preserved", []);
        s.navigate("thread:a");
        const refs =
          intent === "new-refs"
            ? [{ ...s.refs[0], pubkey: "c".repeat(64) }]
            : s.refs;
        if (intent !== "untouched") {
          s.edit(
            intent === "text" || intent === "clear" ? "new A intent" : TEXT,
            refs,
          );
          if (intent === "clear") s.edit("", []);
        }
        s.navigate("thread:b");
        const before = s.store.get("thread:a");
        if (exit === "unmount") s.unmount();
        await s.act(async () => {
          if (outcome === "failure")
            gate.reject(new Error("final validation failed"));
          else gate.resolve();
          await promise;
        });
        assert.equal(s.events("SEND").length, outcome === "success" ? 1 : 0);
        assert.equal(s.options.contentRef.current, "B preserved");
        assert.equal(s.store.get("thread:b").content, "B preserved");
        assert.deepEqual(s.store.get("thread:b").mentionRefs, []);
        if (intent === "untouched" && outcome === "failure") {
          assert.equal(s.store.get("thread:a").content, TEXT);
          assert.deepEqual(s.store.get("thread:a").mentionRefs, s.refs);
        } else {
          assert.deepEqual(
            s.store.get("thread:a"),
            before,
            "new authored value or absence wins",
          );
        }
      });
    }
  }
}

for (const first of ["old", "new"]) {
  test(`new same-key send supersedes old recovery, ${first} completes first`, async () => {
    const s = await setup({ lifecycle: true });
    s.dismiss();
    const oldGate = deferred();
    const old = await ordinary(s, oldGate);
    s.navigate("thread:b");
    s.edit("B preserved", []);
    s.navigate("thread:a");
    s.remount(); // a new composer has its own send latch, sharing draft authority
    // Retry the same captured text without an editor update: claiming a new
    // send itself must revoke the older recovery, even for identical refs.
    s.options.contentRef.current = TEXT;
    s.control.currentRefs = s.refs;
    const newGate = deferred();
    const next = await ordinary(s, newGate);
    assert.equal(s.events("publish").length, 2);
    s.navigate("thread:b");
    const failOld = () =>
      s.act(async () => {
        oldGate.reject(new Error("old validation failed"));
        await old.promise;
      });
    const finishNew = () =>
      s.act(async () => {
        newGate.resolve();
        await next.promise;
      });
    if (first === "old") {
      await failOld();
      await finishNew();
    } else {
      await finishNew();
      await failOld();
    }
    assert.equal(s.events("SEND").length, 1);
    assert.equal(s.store.has("thread:a"), false);
    assert.equal(s.options.contentRef.current, "B preserved");
  });
}

// Media error reaches the same recovery guard as final-validation failure.
test("cross-visit author clear prevents old media error restoring text, refs or files", async () => {
  const s = await setup({ lifecycle: true });
  s.dismiss();
  s.options.mentions.memberPubkeys = new Set([KEY]);
  s.rerender();
  let promise;
  await s.act(async () => {
    promise = s.result.current.sendMessageWithMentionFlow({
      capturedChannelId: "general",
      pendingImeta: [],
      trimmed: TEXT,
      recoveryDraftKey: "thread:a",
      queuedAttachments: [{ id: "old-file" }],
    });
  });
  assert.ok(s.control.uploadCallbacks);
  s.navigate("thread:b");
  s.edit("B preserved", []);
  s.navigate("thread:a");
  s.edit("new A", []);
  s.edit("", []);
  s.navigate("thread:b");
  const queues = s.events("save-queue").length;
  await s.act(async () => {
    s.control.uploadCallbacks.onError(new Error("upload failed"));
    await promise;
  });
  assert.equal(s.store.has("thread:a"), false);
  assert.equal(s.events("save-queue").length, queues);
  assert.equal(s.events("SEND").length, 0);
  assert.equal(s.options.contentRef.current, "B preserved");
});

for (const replacement of ["delete", "same-text", "new-text"]) {
  test(`outgoing lifecycle cleanup cannot overwrite shared ${replacement} from another owner`, async () => {
    const { deleteDraftEntry, saveDraftEntry } = await import(
      "../lib/useDrafts.ts"
    );
    const s = await setup({ lifecycle: true });
    s.dismiss();
    const before = s.store.get("thread:a");
    s.act(() => {
      if (replacement === "delete") deleteDraftEntry("thread:a");
      else
        saveDraftEntry("thread:a", {
          ...before,
          content: replacement === "same-text" ? TEXT : "new owner",
        });
    });
    const expected = s.store.get("thread:a");
    s.unmount();
    assert.deepEqual(s.store.get("thread:a"), expected);
  });
}

test("automatic addressed prefix is programmatic optimistic empty, not new authored intent", async () => {
  const s = await setup({ lifecycle: true });
  s.dismiss();
  s.options.mentions.memberPubkeys = new Set([KEY]);
  s.options.onAddressedAgentsComposerCleared = () => {
    s.options.richText.setContent("@RemoteScout ");
    return "@RemoteScout ";
  };
  const gate = deferred();
  s.control.publish = gate;
  s.rerender();
  const before = s.options.getComposerRevision();
  let promise;
  await s.act(async () => {
    promise = s.result.current.sendMessageWithMentionFlow({
      capturedChannelId: "general",
      pendingImeta: [],
      trimmed: TEXT,
      recoveryDraftKey: "thread:a",
      addressedAgentPubkeys: [KEY],
    });
  });
  assert.equal(
    s.options.getComposerRevision(),
    before + 1,
    "only send claim advances intent",
  );
  await s.act(async () => {
    gate.reject(new Error("send failed"));
    await promise;
  });
  assert.equal(s.options.contentRef.current, TEXT);
  assert.equal(s.store.get("thread:a").content, TEXT);
  assert.deepEqual(s.store.get("thread:a").mentionRefs, s.refs);
});
