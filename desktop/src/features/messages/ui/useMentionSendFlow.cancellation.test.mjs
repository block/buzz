import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";
import * as React from "react";
import ts from "typescript";
import * as helpers from "./useMentionSendFlow.helpers.ts";

// Execute the product hooks with real React effects/renders; only external
// query/mutation/media dependencies are mocked. Deferred promises isolate the
// user-intent boundary independently of successful authorization.
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
before(() =>
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  }),
);
afterEach(async () => (await import("@testing-library/react")).cleanup());
after(() => dom.window.close());
const KEY = "b".repeat(64);
const TEXT = "@RemoteScout hello";
const noop = () => {};
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function load(name, stubs) {
  const source = fs.readFileSync(
    new URL(`./${name}.ts`, import.meta.url),
    "utf8",
  );
  const exports = {};
  vm.runInNewContext(
    ts.transpileModule(source, {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
      },
    }).outputText,
    {
      exports,
      AbortController,
      Error,
      Map,
      Set,
      require: (key) => {
        assert.ok(key in stubs, `unmocked dependency: ${key}`);
        return stubs[key];
      },
    },
  );
  return exports;
}
async function setup({ lifecycle = false } = {}) {
  const { act, renderHook } = await import("@testing-library/react");
  const calls = [];
  const control = { prepare: null, add: null, publish: null, inventory: null };
  const refs = [{ displayName: "RemoteScout", pubkey: KEY, isAgent: true }];
  const query = {
    data: [],
    refetch: async () => {
      if (control.inventory) await control.inventory.promise;
      return { data: [] };
    },
  };
  const mutation = {
    isPending: false,
    mutateAsync: async (input) => {
      calls.push(["add", input]);
      if (control.add) await control.add.promise;
      return { added: [KEY], errors: [] };
    },
  };
  const stubs = {
    react: React,
    sonner: { toast: { error: (error) => calls.push(["error", error]) } },
    "@/features/agents/hooks": new Proxy(
      {},
      {
        get: (_, key) => {
          if (
            key === "useCreateChannelManagedAgentMutation" ||
            key === "useProvisionChannelManagedAgentMutation"
          )
            return () => ({
              isPending: false,
              mutateAsync: async (input) => {
                calls.push(["persona", input]);
                if (control.persona) await control.persona.promise;
                return { agent: { pubkey: KEY, name: "Fizz" } };
              },
            });
          return key.includes("Mutation") ? () => mutation : () => query;
        },
      },
    ),
    "@/features/agents/channelAgents": {
      applyReusableAgentAccessPolicy: async (agent) => {
        calls.push(["local-policy"]);
        if (control.policy) await control.policy.promise;
        return agent;
      },
    },
    "@/features/agents/lib/resolvePersonaRuntime": {
      resolvePersonaRuntime: () => ({ runtime: "test-runtime" }),
    },
    "@/features/channels/hooks": {
      useAddChannelMembersMutation: () => mutation,
    },
    "@/features/channels/useCanAddChannelMembers": {
      useCanAddChannelMembers: () => true,
    },
    "@/features/channels/lib/channelMemberAdmission": {},
    "@/features/messages/lib/dmThreadAgentMentionError": {
      dmThreadAgentMentionError: () => null,
    },
    "@/features/messages/lib/backgroundMediaUploadStore": {
      saveQueuedAttachmentsForDraft: (...args) =>
        calls.push(["save-queue", ...args]),
      prepareBackgroundMediaUpload: () => ({
        start(callbacks) {
          control.uploadCallbacks = callbacks;
          return true;
        },
        cancel: noop,
      }),
    },
    "@/features/messages/lib/imetaMediaMarkdown": {
      buildOutgoingMessage: (text) => ({ content: text, mediaTags: [] }),
    },
    "@/shared/api/tauri": { invokeTauri: async () => {} },
    "@/shared/lib/pubkey": {
      normalizePubkey: (key) => key.toLowerCase(),
      truncatePubkey: (key) => key,
    },
    "@/shared/lib/customEmojiTags": { buildCustomEmojiTags: () => [] },
    "./useMentionSendFlow.helpers": helpers,
    "@/features/messages/lib/agentAddressMention.mjs": {
      buildAgentAddressMentionTags: () => [],
    },
    "@/features/messages/lib/agentMentionRevalidation": {
      AgentMentionAuthorizationError: class extends Error {},
    },
  };
  stubs["./useNonMemberInvite"] = load("useNonMemberInvite", stubs);
  stubs["./useActivePreparedLinkPreviews"] = load(
    "useActivePreparedLinkPreviews",
    stubs,
  );
  const { useMentionSendFlow } = load("useMentionSendFlow", stubs);
  const store = new Map();
  const persistDraft = (
    key,
    content,
    channelId,
    pendingImeta,
    spoileredAttachmentUrls,
    mentionRefs,
  ) => {
    calls.push([
      "persist",
      key,
      content,
      channelId,
      pendingImeta,
      spoileredAttachmentUrls,
      mentionRefs,
    ]);
    if (content || pendingImeta.length)
      store.set(key, {
        content,
        channelId,
        pendingImeta,
        spoileredAttachmentUrls,
        mentionRefs,
      });
    else store.delete(key);
  };
  const initialKey = lifecycle ? "thread:a" : "general";
  if (lifecycle)
    store.set(initialKey, {
      content: TEXT,
      channelId: "general",
      pendingImeta: [],
      spoileredAttachmentUrls: [],
      mentionRefs: refs,
    });
  const options = {
    channelId: "general",
    effectiveDraftKey: initialKey,
    getComposerRevision: () => 0,
    runComposerUpdate: (update) => update(),
    channelType: "stream",
    customEmoji: [],
    mentions: {
      memberPubkeys: new Set(),
      hasResolvedMembers: true,
      extractMentionPersonas: () => [],
      extractMentionPubkeys: () => [KEY],
      isAgentPubkey: (key) => key === KEY,
      isManagedAgentPubkey: () => false,
      getDraftMentionRefs: () => control.currentRefs ?? refs,
      registerMentionPubkey: (displayName, pubkey, options) => {
        const ref = { displayName, pubkey, isAgent: options.isAgent };
        control.currentRefs = [...(control.currentRefs ?? []), ref];
        calls.push(["register-ref", ref]);
      },
      getMentionDisplayName: () => "RemoteScout",
      clearMentions: () => {
        if (lifecycle) control.currentRefs = [];
      },
      restoreDraftMentionRefs: (value) => {
        if (lifecycle) control.currentRefs = value;
        calls.push(["restore-refs", value]);
      },
      revalidateMentionPubkeys: async (keys, channel, opts) => {
        calls.push([opts.phase, channel]);
        if (control[opts.phase]) await control[opts.phase].promise;
        return keys;
      },
    },
    contentRef: { current: TEXT },
    channelLinks: { clearChannels: noop },
    emojiAutocomplete: { clearEmojis: noop },
    richText: {
      clearContent: () => {
        if (lifecycle) lifecycleApi.trackAuthoredContent("");
      },
      setContent: (text) => {
        if (lifecycle) lifecycleApi.trackAuthoredContent(text);
      },
    },
    drafts: {
      loadDraft: (key) => (lifecycle ? store.get(key) : null),
      persistDraft,
      markDraftSent: noop,
    },
    setContent: noop,
    setPendingImeta: noop,
    setIsEmojiPickerOpen: noop,
    clearQueuedAttachments: noop,
    restoreQueuedAttachments: noop,
    hasUnsavedMedia: () => false,
    onSendRef: { current: async (...args) => calls.push(["SEND", ...args]) },
  };
  stubs["@/features/messages/lib/stripImplicitAgentMentions"] = {
    stripImplicitAgentMentionPrefix: (text) => text,
  };
  stubs["@/features/messages/lib/useDrafts"] = {
    getDraftStoreScope: () => "test",
  };
  const { useDraftPersistLifecycle } = load("useDraftPersistSnapshot", stubs);
  let lifecycleApi;
  const hook = renderHook(
    () => {
      if (lifecycle) {
        // biome-ignore lint/correctness/useHookAtTopLevel: lifecycle is immutable for this harness mount
        lifecycleApi = useDraftPersistLifecycle({
          effectiveDraftKey: options.effectiveDraftKey,
          channelId: options.channelId,
          loadDraft: options.drafts.loadDraft,
          persistDraft,
          getMentionRefs: options.mentions.getDraftMentionRefs,
          restoreMentionRefs: options.mentions.restoreDraftMentionRefs,
          livePendingImeta: [],
          setPendingImeta: noop,
          setContent: (text) => {
            options.contentRef.current = text;
          },
          clearContent: () => {
            options.contentRef.current = "";
          },
          setSpoileredAttachmentUrls: noop,
          spoileredAttachmentUrlsRef: { current: new Set() },
          syncComposerContentFromEditor: () => options.contentRef.current,
        });
        options.getComposerRevision = lifecycleApi.getComposerRevision;
        options.runComposerUpdate = lifecycleApi.runComposerUpdate;
      }
      return useMentionSendFlow(options);
    },
    {
      wrapper: ({ children }) =>
        React.createElement(React.StrictMode, null, children),
    },
  );
  const flush = async () =>
    act(async () => {
      await new Promise((resolve) => setImmediate(resolve));
    });
  const prompt = async (text = TEXT) =>
    act(async () => {
      options.contentRef.current = text;
      await hook.result.current.sendMessageWithMentionFlow({
        capturedChannelId: options.channelId,
        pendingImeta: [],
        trimmed: text,
        recoveryDraftKey: options.effectiveDraftKey,
        capturedThreadContext: lifecycle
          ? {
              parentEventId: options.effectiveDraftKey,
              threadHeadId: options.effectiveDraftKey,
            }
          : null,
        queuedAttachments: control.attachments ?? [],
      });
    });
  await prompt();
  const invite = async () =>
    act(async () => hook.result.current.nonMemberPromptProps.onInvite());
  const dismiss = () =>
    act(() => hook.result.current.nonMemberPromptProps.onDismiss());
  const finish = async (gate) => {
    gate.resolve();
    await flush();
  };
  const events = (name) => calls.filter((call) => call[0] === name);
  return {
    ...hook,
    act,
    calls,
    control,
    options,
    query,
    refs,
    prompt,
    invite,
    dismiss,
    finish,
    flush,
    events,
    store,
    edit: (text, mentionRefs = refs) =>
      act(() => {
        options.contentRef.current = text;
        control.currentRefs = mentionRefs;
        lifecycleApi.trackAuthoredContent(text);
      }),
    navigate: (key) => {
      options.effectiveDraftKey = key;
      hook.rerender();
    },
  };
}

for (const stage of ["prepare", "inventory", "add"]) {
  test(`dismiss during delayed ${stage} cannot add further or send, retains draft`, async () => {
    const s = await setup();
    const gate = deferred();
    s.control[stage] = gate;
    if (stage === "inventory") {
      s.query.data = undefined;
      s.rerender();
    }
    await s.invite();
    assert.equal(s.result.current.nonMemberPromptProps.isInvitePending, true);
    s.dismiss();
    assert.equal(s.result.current.nonMemberPromptProps.open, false);
    await s.finish(gate);
    assert.equal(s.events("add").length, stage === "add" ? 1 : 0);
    assert.equal(s.events("SEND").length, 0);
    assert.equal(s.options.contentRef.current, TEXT);
  });
}
for (const action of ["navigation", "unmount", "replacement"]) {
  test(`delayed add ${action} invalidates late completion`, async () => {
    const s = await setup();
    const gate = deferred();
    s.control.add = gate;
    await s.invite();
    assert.equal(s.events("add").length, 1);
    if (action === "navigation") {
      s.options.channelId = "random";
      s.rerender();
    }
    if (action === "unmount") s.unmount();
    if (action === "replacement") await s.prompt("@RemoteScout replacement");
    await s.finish(gate);
    assert.equal(s.events("SEND").length, 0);
    if (action === "replacement") {
      s.control.add = null;
      await s.invite();
      assert.equal(s.events("SEND").length, 1);
      assert.equal(s.events("SEND")[0][1], "@RemoteScout replacement");
    }
  });
}
test("normal Invite survives promotion/render and synchronous double click sends exactly once", async () => {
  const s = await setup();
  const gate = deferred();
  s.control.publish = gate;
  await s.act(async () => {
    s.result.current.nonMemberPromptProps.onInvite();
    s.result.current.nonMemberPromptProps.onInvite();
  });
  assert.equal(s.events("add").length, 1);
  assert.equal(s.events("publish").length, 1);
  assert.equal(s.result.current.nonMemberPromptProps.open, false);
  s.rerender(); // clearing the prompt is NOT cancellation
  await s.finish(gate);
  assert.equal(s.events("SEND").length, 1);
  assert.deepEqual(Array.from(s.events("SEND")[0][2]), [KEY]);
  assert.equal(s.events("SEND")[0][4], "general");
  assert.equal(s.result.current.isPreparingMentionSend, false);
});
for (const action of ["dismissal", "navigation", "unmount"]) {
  test(`signal remains live through final validation: ${action} restores recoverable draft, no send`, async () => {
    const s = await setup();
    const gate = deferred();
    s.control.publish = gate;
    await s.invite();
    assert.equal(s.events("publish").length, 1);
    assert.equal(s.options.contentRef.current, "");
    if (action === "dismissal") s.dismiss();
    if (action === "navigation") {
      s.options.channelId = "random";
      s.rerender();
    }
    if (action === "unmount") s.unmount();
    await s.finish(gate);
    assert.equal(s.events("SEND").length, 0);
    assert.equal(s.events("persist")[0][2], TEXT);
    assert.deepEqual(s.events("persist")[0][6], s.refs);
    if (action === "dismissal") {
      assert.equal(s.options.contentRef.current, TEXT);
      assert.deepEqual(s.events("restore-refs")[0][1], s.refs);
    }
  });
}
test("late cancelled failure cannot reset a newer pending attempt", async () => {
  const s = await setup();
  const old = deferred();
  s.control.add = old;
  await s.invite();
  s.dismiss();
  await s.prompt();
  const current = deferred();
  s.control.add = current;
  await s.invite();
  old.reject(new Error("obsolete add failure"));
  await s.flush();
  assert.equal(s.result.current.nonMemberPromptProps.isInvitePending, true);
  assert.equal(s.result.current.nonMemberPromptProps.error, null);
  await s.finish(current);
  assert.equal(s.events("SEND").length, 1);
});
test("reference-only supersedes preparation and emits no triggering recipient", async () => {
  const s = await setup();
  const old = deferred();
  s.control.prepare = old;
  await s.invite();
  s.control.prepare = null;
  await s.act(async () => s.result.current.nonMemberPromptProps.onDoNothing());
  await s.finish(old);
  assert.equal(s.events("add").length, 0);
  assert.equal(s.events("SEND").length, 1);
  assert.deepEqual(Array.from(s.events("SEND")[0][2]), []);
});

// Readiness is a nested continuation owned by completeSend. Cancelling while
// policy preparation is pending must also stop a subsequent local attachment.
test("cancelled invitation cannot attach a local recipient after delayed policy preparation", async () => {
  const s = await setup();
  s.query.data = [{ pubkey: KEY, name: "LocalScout", status: "running" }];
  s.rerender();
  const gate = deferred();
  s.control.policy = gate;
  await s.invite();
  assert.equal(s.events("local-policy").length, 1);
  s.dismiss();
  await s.finish(gate);
  assert.equal(s.events("add").length, 0);
  assert.equal(s.events("SEND").length, 0);
  assert.equal(s.options.contentRef.current, TEXT);
});

// Real draft lifecycle + real send/Invite hooks share one reused StrictMode host.
// The editor and storage adapter are mocked; draft leave/restore ordering is not.
for (const stage of ["add", "publish"]) {
  for (const incoming of [TEXT, "unrelated thread B draft"]) {
    test(`same-channel ${stage}: incoming ${incoming === TEXT ? "same-text" : "different-text"} draft and exact refs survive`, async () => {
      const s = await setup({ lifecycle: true });
      const otherRefs = [
        { displayName: "RemoteScout", pubkey: "c".repeat(64), isAgent: true },
      ];
      s.store.set("thread:b", {
        content: incoming,
        channelId: "general",
        pendingImeta: [],
        spoileredAttachmentUrls: [],
        mentionRefs: otherRefs,
      });
      const gate = deferred();
      s.control[stage] = gate;
      await s.invite();
      s.navigate("thread:b");
      assert.equal(s.result.current.nonMemberPromptProps.open, false);
      assert.equal(s.result.current.isPreparingMentionSend, false);
      assert.equal(s.options.contentRef.current, incoming);
      assert.deepEqual(s.control.currentRefs, otherRefs);
      // Recovery must be available on return BEFORE the network responds.
      s.navigate("thread:a");
      assert.equal(s.options.contentRef.current, TEXT);
      assert.deepEqual(s.control.currentRefs, s.refs);
      s.edit(TEXT, otherRefs); // identical visible text is a new exact selection
      s.navigate("thread:b");
      await s.finish(gate);
      assert.equal(s.events("SEND").length, 0);
      assert.equal(s.options.contentRef.current, incoming);
      assert.deepEqual(s.control.currentRefs, otherRefs);
      assert.deepEqual(s.store.get("thread:a").mentionRefs, otherRefs);
      assert.deepEqual(s.store.get("thread:b").mentionRefs, otherRefs);
    });
  }
}
test("late validation completion cannot restore over an authored empty draft or reset a newer send", async () => {
  const s = await setup({ lifecycle: true });
  const old = deferred();
  s.control.publish = old;
  await s.invite();
  s.edit("new text");
  s.edit("");
  s.dismiss();
  assert.equal(s.options.contentRef.current, "");
  const current = deferred();
  s.control.publish = current;
  await s.prompt();
  await s.invite();
  await s.finish(old);
  assert.equal(s.result.current.isPreparingMentionSend, true);
  assert.equal(s.events("SEND").length, 0);
  await s.finish(current);
  assert.equal(s.events("SEND").length, 1);
});
test("cancelled media continuation preserves refs and cannot overwrite an unrelated stored draft", async () => {
  const s = await setup({ lifecycle: true });
  s.dismiss();
  s.control.attachments = [{ id: "file", file: {}, spoilered: false }];
  await s.prompt();
  await s.invite();
  assert.ok(s.control.uploadCallbacks);
  const other = {
    content: "new saved draft",
    channelId: "general",
    pendingImeta: [],
    spoileredAttachmentUrls: [],
    mentionRefs: [],
  };
  s.store.set("thread:a", other);
  s.dismiss();
  assert.equal(s.options.contentRef.current, TEXT);
  assert.deepEqual(s.control.currentRefs, s.refs);
  assert.equal(s.store.get("thread:a"), other);
  await s.act(async () =>
    s.control.uploadCallbacks.onComplete([], new AbortController().signal),
  );
  assert.equal(s.events("SEND").length, 0);
  assert.equal(s.store.get("thread:a"), other);
});

for (const action of ["unchanged", "navigate", "edit"]) {
  test(`delayed persona reuse binds the captured draft, not a later selection: ${action}`, async () => {
    const s = await setup({ lifecycle: true });
    s.dismiss();
    const text = "@Fizz hello";
    const personaRefs = [{ displayName: "Fizz", pubkey: KEY, isAgent: true }];
    const otherRefs = [{ ...personaRefs[0], pubkey: "c".repeat(64) }];
    s.edit(text, []);
    s.store.delete("thread:a"); // new, not-yet-persisted persona draft
    s.options.mentions.extractMentionPubkeys = () => [];
    s.options.mentions.extractMentionPersonas = () => [
      {
        displayName: "Fizz",
        persona: { id: "builtin:fizz", displayName: "Fizz" },
      },
    ];
    const gate = deferred();
    s.control.persona = gate;
    s.rerender();
    let send;
    await s.act(async () => {
      send = s.result.current.sendMessageWithMentionFlow({
        capturedChannelId: "general",
        pendingImeta: [],
        trimmed: text,
        recoveryDraftKey: "thread:a",
      });
    });
    assert.equal(s.events("persona").length, 1);
    if (action === "navigate") {
      s.store.set("thread:b", {
        content: text,
        channelId: "general",
        pendingImeta: [],
        spoileredAttachmentUrls: [],
        mentionRefs: otherRefs,
      });
      s.navigate("thread:b");
    } else if (action === "edit") s.edit(text, otherRefs);
    // Transport failure owes exact resolved persona refs to the source snapshot,
    // without replacing a new editor's identical-label selection.
    s.options.onSendRef.current = async () => {
      throw new Error("transport");
    };
    await s.finish(gate);
    await send;
    if (action === "unchanged") {
      assert.equal(s.events("register-ref").length, 1);
      assert.equal(s.options.contentRef.current, text);
      assert.deepEqual(
        JSON.parse(JSON.stringify(s.control.currentRefs)),
        personaRefs,
      );
      assert.deepEqual(
        JSON.parse(JSON.stringify(s.store.get("thread:a").mentionRefs)),
        personaRefs,
      );
    } else {
      assert.equal(s.events("register-ref").length, 0);
      assert.equal(s.options.contentRef.current, text);
      assert.deepEqual(s.control.currentRefs, otherRefs);
    }
  });
}

// Independent reviewer reproduction, plus storage/queued-file boundaries.
for (const media of [false, true]) {
  for (const action of ["dismiss", "navigate", "unmount"]) {
    test(`authored edit-clear before ${action} stays deleted (queued media: ${media})`, async () => {
      const s = await setup({ lifecycle: true });
      const gate = deferred();
      s.control.publish = gate;
      if (media) {
        s.dismiss();
        s.control.attachments = [{ id: "file", file: {}, spoilered: false }];
        await s.prompt();
      }
      await s.invite();
      let upload;
      if (media) {
        await s.act(async () => {
          upload = s.control.uploadCallbacks.onComplete(
            [],
            new AbortController().signal,
          );
        });
      }
      assert.equal(s.events("publish").length, 1);
      s.edit("new authored text");
      s.edit("", []);
      assert.equal(s.store.has("thread:a"), false);
      const recoveryWrites = s.events("save-queue").length;
      if (action === "navigate") s.navigate("thread:b");
      else if (action === "unmount") s.unmount();
      else s.dismiss();
      assert.equal(s.store.has("thread:a"), false);
      if (action === "navigate") {
        s.edit(TEXT, [{ ...s.refs[0], pubkey: "c".repeat(64) }]);
        s.navigate("thread:a");
        assert.equal(s.options.contentRef.current, "");
        assert.equal(s.control.currentRefs.length, 0);
        s.edit("new A after return", []);
        s.navigate("thread:b");
      }
      await s.finish(gate);
      if (upload) await upload;
      assert.equal(s.events("SEND").length, 0);
      assert.equal(s.events("save-queue").length, recoveryWrites);
      if (action === "navigate") {
        assert.equal(s.options.contentRef.current, TEXT);
        assert.equal(s.control.currentRefs[0].pubkey, "c".repeat(64));
        assert.equal(s.store.get("thread:a").content, "new A after return");
        assert.deepEqual(s.store.get("thread:a").mentionRefs, []);
      } else assert.equal(s.store.has("thread:a"), false);
    });
  }
}
for (const incoming of [TEXT, "different B text"]) {
  for (const edit of ["untouched", "same-text-new-refs", "new-text"]) {
    test(`source authority survives navigation: ${edit}, ${incoming}`, async () => {
      const s = await setup({ lifecycle: true });
      const otherRefs = [{ ...s.refs[0], pubkey: "c".repeat(64) }];
      s.store.set("thread:b", {
        content: incoming,
        channelId: "general",
        pendingImeta: [],
        spoileredAttachmentUrls: [],
        mentionRefs: otherRefs,
      });
      const gate = deferred();
      s.control.publish = gate;
      await s.invite();
      if (edit !== "untouched")
        s.edit(edit === "new-text" ? "new A text" : TEXT, otherRefs);
      s.navigate("thread:b");
      const saved = s.store.get("thread:a");
      assert.equal(saved.content, edit === "new-text" ? "new A text" : TEXT);
      assert.deepEqual(
        saved.mentionRefs,
        edit === "untouched" ? s.refs : otherRefs,
      );
      s.edit("B edit during old await", []);
      await s.finish(gate);
      assert.equal(s.store.get("thread:a"), saved);
      assert.equal(s.options.contentRef.current, "B edit during old await");
      assert.equal(s.control.currentRefs.length, 0);
      assert.equal(s.events("SEND").length, 0);
    });
  }
}

for (const action of ["untouched", "deleted", "superseded"]) {
  test(`ordinary send preflight recovery respects source authority on unmount: ${action}`, async () => {
    const s = await setup({ lifecycle: true });
    s.dismiss();
    s.options.mentions.memberPubkeys = new Set([KEY]);
    const gate = deferred();
    s.control.prepare = gate;
    s.rerender();
    let send;
    await s.act(async () => {
      send = s.result.current.sendMessageWithMentionFlow({
        capturedChannelId: "general",
        pendingImeta: [],
        trimmed: TEXT,
        recoveryDraftKey: "thread:a",
      });
    });
    assert.equal(s.options.contentRef.current, "");
    if (action === "deleted") {
      s.edit("new text");
      s.edit("", []);
    }
    s.unmount();
    const other = {
      content: "new saved draft",
      channelId: "general",
      pendingImeta: [],
      spoileredAttachmentUrls: [],
      mentionRefs: [],
    };
    if (action === "superseded") s.store.set("thread:a", other);
    await s.finish(gate);
    await send;
    assert.equal(s.events("SEND").length, 0);
    if (action === "deleted") assert.equal(s.store.has("thread:a"), false);
    else if (action === "superseded")
      assert.equal(s.store.get("thread:a"), other);
    else {
      assert.equal(s.store.get("thread:a").content, TEXT);
      assert.deepEqual(s.store.get("thread:a").mentionRefs, s.refs);
    }
  });
}
for (const authored of [false, true]) {
  test(`sent-draft cleanup consults exited source visit, not B revision (authored: ${authored})`, async () => {
    const s = await setup({ lifecycle: true });
    s.dismiss();
    s.options.mentions.memberPubkeys = new Set([KEY]);
    s.options.drafts.markDraftSent = (...args) =>
      s.calls.push(["mark-sent", ...args]);
    const gate = deferred();
    s.control.publish = gate;
    s.rerender();
    let send;
    await s.act(async () => {
      send = s.result.current.sendMessageWithMentionFlow({
        capturedChannelId: "general",
        pendingImeta: [],
        trimmed: TEXT,
        recoveryDraftKey: "thread:a",
        sentDraftKey: "thread:a",
      });
    });
    if (authored) s.edit(TEXT); // same snapshot, but a genuinely newer draft
    const sourceRevision = s.options.getComposerRevision;
    const revision = sourceRevision();
    s.navigate("thread:b");
    s.edit("B edit", []);
    assert.equal(
      sourceRevision(),
      revision,
      "B must not change the captured A revision",
    );
    // An untouched optimistic empty has no stored refs. Seed the unchanged
    // submitted snapshot to exercise markDraftSent's bounded exact-ref guard.
    if (!authored)
      s.store.set("thread:a", {
        content: TEXT,
        channelId: "general",
        pendingImeta: [],
        spoileredAttachmentUrls: [],
        mentionRefs: s.refs,
      });
    await s.finish(gate);
    await send;
    assert.equal(s.events("SEND").length, 1); // ordinary destination-bound send
    assert.equal(s.events("mark-sent").length, authored ? 0 : 1);
    assert.equal(s.options.contentRef.current, "B edit");
    assert.equal(s.control.currentRefs.length, 0);
  });
}
