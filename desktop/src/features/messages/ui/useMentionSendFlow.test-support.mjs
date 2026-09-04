import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { after, afterEach, before } from "node:test";
import { JSDOM } from "jsdom";
import * as React from "react";
import ts from "typescript";
import * as helpers from "./useMentionSendFlow.helpers.ts";
import * as draftStore from "../lib/useDrafts.ts";

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
    localStorage: dom.window.localStorage,
  }),
);
afterEach(async () => (await import("@testing-library/react")).cleanup());
after(() => dom.window.close());
export const KEY = "b".repeat(64);
export const TEXT = "@RemoteScout hello";
const noop = () => {};
export function deferred() {
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
export async function setup({ lifecycle = false } = {}) {
  const { act, renderHook } = await import("@testing-library/react");
  draftStore.clearAllDrafts();
  dom.window.localStorage.clear();
  draftStore.initDraftStore("test-author", "wss://test.example");
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
    "@/features/messages/lib/useDrafts": draftStore,
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
    "@/features/communities/useCommunities": {
      useCommunities: () => ({
        activeCommunity: { relayUrl: "wss://test.example" },
      }),
    },
    "@/shared/api/hooks": {
      useIdentityQuery: () => ({ data: { pubkey: "a".repeat(64) } }),
    },
    "@/features/messages/lib/detachedToastScope": {
      matchesDetachedToastScope: () => true,
    },
    "@/features/agents/channelAgents": {
      applyReusableAgentAccessPolicy: async (agent) => {
        calls.push(["local-policy"]);
        if (control.policy) await control.policy.promise;
        return { agent, wrote: false };
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
  stubs["./useDetachedAgentStart"] = load("useDetachedAgentStart", stubs);
  stubs["./useEnsureAgentMentionsReady"] = load(
    "useEnsureAgentMentionsReady",
    stubs,
  );
  stubs["./useNonMemberInvite"] = load("useNonMemberInvite", stubs);
  stubs["./useActivePreparedLinkPreviews"] = load(
    "useActivePreparedLinkPreviews",
    stubs,
  );
  const { useMentionSendFlow } = load("useMentionSendFlow", stubs);
  // Real draft adapter: empty persistence deletes the actual value, while
  // shared semantic authority remains independently readable.
  const store = {
    get: draftStore.loadDraftEntry,
    has: (key) => draftStore.loadDraftEntry(key) !== undefined,
    set: (key, value) =>
      draftStore.persistDraftEntry(
        key,
        value.content,
        value.channelId,
        value.pendingImeta,
        value.spoileredAttachmentUrls,
        value.mentionRefs,
      ),
    delete: draftStore.clearDraftEntry,
  };
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
      settlePendingMentionBindings: async () => {},
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
      markDraftSent: draftStore.markDraftSentEntry,
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
  const { useDraftPersistLifecycle } = load("useDraftPersistSnapshot", stubs);
  let lifecycleApi;
  const renderComposer = () => {
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
  };
  const mount = () =>
    renderHook(renderComposer, {
      wrapper: ({ children }) =>
        React.createElement(React.StrictMode, null, children),
    });
  let hook = mount();
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
    get result() {
      return hook.result;
    },
    rerender: () => hook.rerender(),
    unmount: () => hook.unmount(),
    remount: () => {
      hook.unmount();
      hook = mount();
    },
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
