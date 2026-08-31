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
async function setup() {
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
        get: (_, key) =>
          key.includes("Mutation") ? () => mutation : () => query,
      },
    ),
    "@/features/agents/channelAgents": {
      applyReusableAgentAccessPolicy: async (agent) => {
        calls.push(["local-policy"]);
        if (control.policy) await control.policy.promise;
        return agent;
      },
    },
    "@/features/agents/lib/resolvePersonaRuntime": {},
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
      saveQueuedAttachmentsForDraft: noop,
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
  const options = {
    channelId: "general",
    channelType: "stream",
    customEmoji: [],
    mentions: {
      memberPubkeys: new Set(),
      hasResolvedMembers: true,
      extractMentionPersonas: () => [],
      extractMentionPubkeys: () => [KEY],
      isAgentPubkey: (key) => key === KEY,
      isManagedAgentPubkey: () => false,
      getDraftMentionRefs: () => refs,
      getMentionDisplayName: () => "RemoteScout",
      clearMentions: noop,
      restoreDraftMentionRefs: (value) => calls.push(["restore-refs", value]),
      revalidateMentionPubkeys: async (keys, channel, opts) => {
        calls.push([opts.phase, channel]);
        if (control[opts.phase]) await control[opts.phase].promise;
        return keys;
      },
    },
    contentRef: { current: TEXT },
    channelLinks: { clearChannels: noop },
    emojiAutocomplete: { clearEmojis: noop },
    richText: { clearContent: noop, setContent: noop },
    drafts: {
      loadDraft: () => null,
      persistDraft: (...args) => calls.push(["persist", ...args]),
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
  const hook = renderHook(() => useMentionSendFlow(options), {
    wrapper: ({ children }) =>
      React.createElement(React.StrictMode, null, children),
  });
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
        recoveryDraftKey: "general",
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
