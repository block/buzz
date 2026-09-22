import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";
import * as React from "react";
import ts from "typescript";
import * as draftStore from "../../messages/lib/useDrafts.ts";

const clients = [];
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
  pretendToBeVisual: true,
});
// JSDOM has no layout; these shims support ProseMirror scrolling, not selection.
dom.window.Range.prototype.getClientRects = () => [];
dom.window.Range.prototype.getBoundingClientRect = () =>
  new dom.window.DOMRect();
before(() =>
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
    localStorage: dom.window.localStorage,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Element: dom.window.Element,
    MutationObserver: dom.window.MutationObserver,
    ResizeObserver: class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
    getComputedStyle: dom.window.getComputedStyle,
    requestAnimationFrame: (callback) => setTimeout(callback, 0),
    cancelAnimationFrame: clearTimeout,
    IS_REACT_ACT_ENVIRONMENT: true,
  }),
);
afterEach(async () => {
  (await import("@testing-library/react")).cleanup();
  for (const client of clients.splice(0)) client.clear();
});
after(() => dom.window.close());
const KEY = "b".repeat(64);
const REFS = [{ displayName: "RemoteScout", pubkey: KEY, isAgent: true }];
const MEDIA = [
  {
    url: "https://media.example/file.pdf",
    type: "application/pdf",
    filename: "file.pdf",
    sha256: "a".repeat(64),
    size: 123,
    uploaded: true,
  },
];
const TEXT = "@RemoteScout hello";
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function load(relative, stubs) {
  const source = fs.readFileSync(new URL(relative, import.meta.url), "utf8");
  const exports = {};
  vm.runInNewContext(
    ts.transpileModule(source, {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
        jsx: ts.JsxEmit.React,
      },
    }).outputText,
    {
      exports,
      Error,
      Set,
      Map,
      window,
      document,
      Node,
      setTimeout,
      clearTimeout,
      require: (name) => {
        assert.ok(name in stubs, `unmocked dependency: ${name}`);
        return stubs[name];
      },
    },
  );
  return exports;
}
// Load the production owner with only its directory/network hooks replaced.
async function loadWithImports(relative, overrides) {
  const url = new URL(relative, import.meta.url);
  const compiled = ts.transpileModule(fs.readFileSync(url, "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.React },
  }).outputText;
  const dependencies = { ...overrides };
  for (const [, name] of compiled.matchAll(/require\("([^"]+)"\)/g)) {
    if (!(name in dependencies))
      dependencies[name] = await import(
        name.startsWith(".")
          ? new URL(
              `${name}${fs.existsSync(new URL(`${name}.ts`, url)) ? ".ts" : ".tsx"}`,
              url,
            ).href
          : name
      );
  }
  return load(relative, dependencies);
}
async function setup(options = {}) {
  const { persistent = true } = options;
  const channelType = "channelType" in options ? options.channelType : "forum";
  const { act, render, fireEvent } = await import("@testing-library/react");
  draftStore.clearAllDrafts();
  localStorage.clear();
  (
    await import("../../messages/lib/persistentAgentAudience.ts")
  ).resetPersistentAgentAudienceStore();
  draftStore.initDraftStore("forum-test", "wss://forum.test");
  const calls = [];
  const control = { add: null, publish: null, prepare: null };
  const noop = () => {};
  let editor, media, mentionState, prompt;
  const stubs = {
    react: React,
    sonner: { toast: { error: (value) => calls.push(["error", value]) } },
    "@tiptap/react": { EditorContent: () => null },
    "lucide-react": { ChevronDown: () => null },
    "@/shared/lib/cn": { cn: () => "" },
    "@/shared/lib/pubkey": {
      normalizePubkey: (s) => s.toLowerCase(),
      truncatePubkey: (s) => s,
      // Compact-identity seam stubbed alongside its sibling: this suite
      // renders display names, never key-form labels.
      truncateNpub: (s) => s,
    },
    "@/features/channels/hooks": {
      useAddChannelMembersMutation: () => ({
        mutateAsync: async () => {
          calls.push(["add"]);
          if (control.add) await control.add.promise;
          return { errors: [] };
        },
      }),
    },
    "@/features/channels/useCanAddChannelMembers": {
      useCanAddChannelMembers: () => true,
    },
    "@/features/channels/lib/channelMemberAdmission": {
      PRIVATE_CHANNEL_ADD_DENIED_MESSAGE: "denied",
    },
    "@/features/messages/lib/useDrafts": draftStore,
    "@/features/messages/lib/stripImplicitAgentMentions": {
      stripImplicitAgentMentionPrefix: (s) => s,
    },
    "@/features/messages/lib/useComposerFocusOwnership": {
      useComposerFocusOwnership: () => true,
    },
    "@/features/messages/lib/useChannelLinks": {
      useChannelLinks: () => ({
        clearChannels: noop,
        updateChannelQuery: noop,
        handleChannelKeyDown: () => ({ handled: false }),
      }),
    },
    "@/features/messages/lib/mentionCodeContext": {
      isMentionCodeContext: () => false,
    },
    "@/features/messages/lib/normalizeMentionClipboard": {
      hasMentionClipboardHtml: () => false,
    },
    "@/features/messages/lib/mentionClipboardPaste": {
      handleMentionClipboardPaste: () => false,
    },
    "@/features/messages/lib/useLinkEditor": { useLinkEditor: () => ({}) },
    "./useCompactComposerInteractions": {
      useCompactComposerInteractions: () => ({ shouldIgnoreBlur: () => false }),
    },
    "@/features/messages/lib/useMentions": {
      useMentions: () => {
        const state = React.useRef({ refs: [] });
        mentionState = state.current;
        return {
          knownNames: {},
          settlePendingMentionBindings: async () => {
            if (control.settle) await control.settle.promise;
          },
          cancelMentionAutocomplete: noop,
          updateMentionQuery: noop,
          clearMentions: () => {
            state.current.refs = [];
          },
          restoreDraftMentionRefs: (refs) => {
            state.current.refs = [...refs];
          },
          getDraftMentionRefs: () => state.current.refs,
          extractMentionPubkeys: () =>
            state.current.refs.map((ref) => ref.pubkey),
          isAgentPubkey: () => true,
          isManagedAgentPubkey: () => false,
          hasResolvedMembers: true,
          memberPubkeys: new Set(),
          getMentionDisplayName: () => "RemoteScout",
          revalidateMentionPubkeys: async (keys, _channel, { phase }) => {
            calls.push([phase]);
            if (control[phase]) await control[phase].promise;
            return keys;
          },
        };
      },
    },
    "@/features/messages/lib/useMediaUpload": {
      useMediaUpload: () => {
        const [pendingImeta, setPendingImeta] = React.useState([]);
        const ref = React.useRef(pendingImeta);
        ref.current = pendingImeta;
        media = {
          pendingImeta,
          pendingImetaRef: ref,
          setPendingImeta,
          uploadState: { status: "idle" },
          queuedAttachments: [],
          queuedAttachmentsRef: { current: [] },
          clearQueuedAttachments: noop,
          restoreQueuedAttachments: noop,
          setUploadState: noop,
        };
        return media;
      },
    },
    "@/features/messages/lib/useRichTextEditor": {
      useRichTextEditor: (options) => {
        const state = React.useRef("");
        editor = {
          getMarkdown: () => state.current,
          setContent: (value) => {
            state.current = value;
          },
          clearContent: () => {
            state.current = "";
          },
          edit: (value) => {
            state.current = value;
            options.onUpdate({ text: value, cursor: value.length });
          },
          focus: noop,
        };
        return editor;
      },
    },
    "@/features/messages/lib/imetaMediaMarkdown": {
      buildOutgoingMessage: (content, imeta) => ({ content, mediaTags: imeta }),
    },
    "@/features/messages/ui/NonMemberMentionDialog": {
      NonMemberMentionDialog: (props) => {
        prompt = props;
        return null;
      },
    },
  };
  for (const [path, names] of [
    ["@/shared/ui/button", ["Button"]],
    [
      "@/shared/ui/dropdown-menu",
      [
        "DropdownMenu",
        "DropdownMenuContent",
        "DropdownMenuRadioGroup",
        "DropdownMenuRadioItem",
        "DropdownMenuTrigger",
      ],
    ],
    ["@/features/messages/ui/ComposerAttachments", ["DropZoneOverlay"]],
    [
      "@/features/messages/ui/MessageComposerToolbar",
      ["MessageComposerToolbar"],
    ],
    ["./ForumComposerAutocompletes", ["ForumComposerAutocompletes"]],
    ["./ForumComposerCompactLayout", ["ForumComposerCompactLayout"]],
    ["./ForumComposerMediaStatus", ["ForumComposerMediaStatus"]],
  ])
    stubs[path] = Object.fromEntries(names.map((name) => [name, () => null]));
  let queryClient;
  if (options.realMentions) {
    const { QueryClient } = await import("@tanstack/react-query");
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    clients.push(queryClient);
    const query = (read) => ({
      data: read(),
      isSuccess: true,
      isFetched: true,
      error: null,
      refetch: async () => ({ data: read(), error: null }),
    });
    control.personas = [
      { id: "persona", displayName: "Scout", isActive: true },
    ];
    control.agents = [
      { pubkey: KEY, name: "Scout", personaId: "persona", status: "running" },
    ];
    control.teams = [{ id: "team", name: "Crew", personaIds: ["persona"] }];
    const agentHooks = {
      useManagedAgentsQuery: () => query(() => control.agents),
      useRelayAgentsQuery: () => query(() => []),
      usePersonasQuery: () => ({
        ...query(() => control.personas),
        refetch: async () => {
          if (control.definition) await control.definition.promise;
          return { data: control.personas, error: null };
        },
      }),
      useTeamsQuery: () => query(() => control.teams),
    };
    const revalidation = await loadWithImports(
      "../../messages/lib/agentMentionRevalidation.ts",
      {
        "@/shared/api/tauriRelayAgents": {
          revalidateRelayAgents: async () => {
            calls.push(["selection-prepare"]);
            if (control.selection) await control.selection.promise;
            return [];
          },
        },
      },
    );
    const productionMentions = await loadWithImports(
      "../../messages/lib/useMentions.ts",
      {
        "@/features/agents/hooks": agentHooks,
        "@/features/channels/hooks": {
          useChannelMembersQuery: () => query(() => []),
          useChannelsQuery: () => query(() => []),
        },
        "@/features/identity-archive/hooks": {
          useIsArchivedPredicate: () => () => false,
        },
        "@/shared/api/hooks": {
          useIdentityQuery: () => query(() => ({ pubkey: "a".repeat(64) })),
        },
        "@/features/profile/hooks": {
          useInfiniteUserSearchQuery: () => query(() => ({ pages: [] })),
          useUsersBatchQuery: () => query(() => ({ profiles: {} })),
        },
        "./agentMentionRevalidation": revalidation,
      },
    );
    stubs["@/features/messages/lib/useMentions"] = {
      useMentions: (...args) => {
        const result = productionMentions.useMentions(...args);
        mentionState = result;
        return result;
      },
    };
    const realEditor = await import("../../messages/lib/useRichTextEditor.ts");
    stubs["@/features/messages/lib/useRichTextEditor"] = {
      useRichTextEditor: (opts) => {
        editor = realEditor.useRichTextEditor(opts);
        return editor;
      },
    };
    stubs["@tiptap/react"] = await import("@tiptap/react");
    stubs["@/features/messages/lib/useComposerFocusOwnership"] = await import(
      "../../messages/lib/useComposerFocusOwnership.ts"
    );
    stubs["./ForumComposerAutocompletes"] = await import(
      "./ForumComposerAutocompletes.tsx"
    );
  }
  stubs["@/features/messages/ui/useDraftPersistSnapshot"] = load(
    "../../messages/ui/useDraftPersistSnapshot.ts",
    stubs,
  );
  stubs["./useForumMentionPreparation"] = load(
    "./useForumMentionPreparation.ts",
    stubs,
  );
  stubs["./useForumDraftRecovery"] = load("./useForumDraftRecovery.ts", stubs);
  stubs["@/features/messages/lib/useMentionAdmissionEditor"] = await import(
    "../../messages/lib/useMentionAdmissionEditor.ts"
  );
  let Composer = load("./ForumComposer.tsx", stubs).ForumComposer;
  if (options.main) {
    Object.assign(stubs, {
      "@/features/messages/lib/useComposerSpoilerParticles": {
        useComposerSpoilerParticles: noop,
      },
      "@/features/custom-emoji/hooks": { useCustomEmoji: () => [] },
      "@/shared/api/hooks": {
        useIdentityQuery: () => ({ data: { pubkey: "a".repeat(64) } }),
      },
      "@/features/messages/useTypingBroadcast": {
        useTypingBroadcast: () => noop,
      },
      "@/features/messages/lib/imetaMediaMarkdown": await import(
        "../../messages/lib/imetaMediaMarkdown.ts"
      ),
      "./useComposerVoiceNote": {
        useComposerVoiceNote: () => ({
          acceptsAttachment: true,
          hasAttachmentRef: { current: false },
        }),
      },
      "./ComposerDockToolbar": { ComposerDockToolbar: () => null },
      "./ComposerReplyEditBanner": { ComposerReplyEditBanner: () => null },
      "./ComposerUploadProgressPill": {
        ComposerUploadProgressPill: () => null,
      },
    });
    stubs["./useDetachedAgentStart"] = { useDetachedAgentStart: () => ({}) };
    stubs["./useMentionSendFlow"] = await loadWithImports(
      "../../messages/ui/useMentionSendFlow.ts",
      stubs,
    );
    Composer = (
      await loadWithImports("../../messages/ui/MessageComposer.tsx", stubs)
    ).MessageComposer;
  }
  let source = "a";
  let transport;
  const props = () => ({
    channelId: channelType === "forum" ? "forum" : null,
    channelType,
    draftKey: persistent
      ? `${options.keyPrefix ?? "thread"}:${source}`
      : undefined,
    placeholder: "Reply",
    audienceContext: options.main
      ? { rootTags: [], rootEventId: "root" }
      : undefined,
    onSubmit: async (...args) => {
      calls.push(["send", source, ...args]);
      if (transport) await transport.promise;
    },
  });
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { TooltipProvider } = await import("@/shared/ui/tooltip");
  const wrap = (child) =>
    queryClient
      ? React.createElement(
          QueryClientProvider,
          { client: queryClient },
          React.createElement(TooltipProvider, null, child),
        )
      : child;
  const view = render(
    wrap(
      React.createElement(
        React.StrictMode,
        null,
        React.createElement(Composer, props()),
      ),
    ),
  );
  const flush = () => act(async () => {});
  return {
    calls,
    control,
    view,
    get richText() {
      return editor;
    },
    get mentions() {
      return mentionState;
    },
    async open() {
      act(() => editor.editor.view.dom.focus());
      act(() =>
        mentionState.openMentionPicker(editor.getPlainTextAndCursor().cursor),
      );
      await flush();
    },
    async choose(label) {
      const row = view.getByRole("button", {
        name: `Mention ${label}`,
        exact: true,
      });
      act(() => fireEvent.mouseDown(row));
      act(() => fireEvent.click(row));
      await flush();
    },
    get prompt() {
      return prompt;
    },
    get text() {
      return editor.getMarkdown();
    },
    get refs() {
      return mentionState.refs;
    },
    get imeta() {
      return media.pendingImeta;
    },
    set transport(value) {
      transport = value;
    },
    edit(text = TEXT, refs = REFS) {
      act(() => {
        mentionState.refs = refs;
        editor.edit(text);
      });
    },
    attach(value = MEDIA) {
      act(() => media.setPendingImeta(value));
    },
    deleteDraft() {
      act(() =>
        draftStore.deleteDraftEntry(
          `${options.keyPrefix ?? "thread"}:${source}`,
        ),
      );
    },
    async submit() {
      act(() => fireEvent.submit(view.container.querySelector("form")));
      await flush();
    },
    async invite() {
      act(() => prompt.onInvite());
      await flush();
    },
    async dismiss() {
      act(() => prompt.onDismiss());
      await flush();
    },
    navigate(value) {
      source = value;
      view.rerender(
        wrap(
          React.createElement(
            React.StrictMode,
            null,
            React.createElement(Composer, props()),
          ),
        ),
      );
    },
    async finish(gate, error) {
      await act(async () => (error ? gate.reject(error) : gate.resolve()));
    },
    unmount: view.unmount,
  };
}
for (const stage of ["prepare", "add", "publish"]) {
  test(`production composer ${stage}: source visit retains refs/media, no late publication or error`, async () => {
    const s = await setup();
    s.edit();
    s.attach();
    await s.submit();
    const gate = deferred();
    s.control[stage] = gate;
    await s.invite();
    s.navigate("b");
    assert.equal(s.text, "");
    s.edit("B draft", []);
    s.navigate("a");
    assert.equal(s.text, TEXT);
    assert.deepEqual(s.refs, REFS);
    assert.deepEqual(Array.from(s.imeta), MEDIA);
    s.edit("", []);
    s.navigate("b");
    await s.finish(gate, new Error("obsolete failure"));
    assert.equal(s.text, "B draft");
    assert.equal(
      s.calls.filter((c) => c[0] === "send" || c[0] === "error").length,
      0,
    );
    assert.equal(
      s.calls.filter((c) => c[0] === "add").length,
      stage === "prepare" ? 0 : 1,
    );
    s.navigate("a");
    assert.equal(s.text, "");
    assert.deepEqual(s.refs, []);
  });
}
test("cancel/retry owns its latch: old add cannot release or fail a new pending invitation", async () => {
  const s = await setup();
  s.edit();
  await s.submit();
  const old = deferred();
  s.control.add = old;
  await s.invite();
  await s.dismiss();
  await s.submit();
  const current = deferred();
  s.control.add = current;
  await s.invite();
  await s.finish(old, new Error("old error"));
  assert.equal(s.prompt.isInvitePending, true);
  assert.equal(s.prompt.error, null);
  await s.finish(current);
  assert.equal(s.calls.filter((c) => c[0] === "send").length, 1);
  assert.equal(s.text, "");
});
for (const channelType of [null, undefined]) {
  test(`shared channel-less consumer ${channelType}: failed transport retains content/media/refs for exact retry`, async () => {
    const s = await setup({ channelType, persistent: false });
    s.edit();
    s.attach();
    const transport = deferred();
    s.transport = transport;
    await s.submit();
    assert.equal(s.prompt.open, false);
    assert.equal(s.text, "");
    await s.finish(transport, new Error("transport failed"));
    assert.equal(s.text, TEXT);
    assert.deepEqual(s.refs, REFS);
    assert.deepEqual(Array.from(s.imeta), MEDIA);
    s.transport = null;
    await s.submit();
    assert.equal(s.text, "");
    assert.equal(s.calls.filter((c) => c[0] === "add").length, 0);
    assert.deepEqual(
      Array.from(s.calls.filter((c) => c[0] === "send").at(-1)[3]),
      [KEY],
    );
  });
}

test("dismiss during final validation invalidates publication but retains the source draft", async () => {
  const s = await setup();
  s.edit();
  s.attach();
  await s.submit();
  const gate = deferred();
  s.control.publish = gate;
  await s.invite();
  assert.equal(s.prompt.open, false);
  await s.dismiss();
  await s.finish(gate);
  assert.equal(s.text, TEXT);
  assert.deepEqual(s.refs, REFS);
  assert.deepEqual(Array.from(s.imeta), MEDIA);
  assert.equal(s.calls.filter((c) => c[0] === "send").length, 0);
});

test("late transport failure never restores into a later source visit", async () => {
  const s = await setup();
  s.edit();
  await s.submit();
  const transport = deferred();
  s.transport = transport;
  await s.invite();
  assert.equal(s.text, "");
  s.navigate("b");
  s.edit("B stays", []);
  s.navigate("a");
  s.edit("new A intent", []);
  await s.finish(transport, new Error("old transport failed"));
  assert.equal(s.text, "new A intent");
  assert.deepEqual(s.refs, []);
  assert.equal(s.calls.filter((c) => c[0] === "error").length, 0);
});

for (const keyPrefix of ["thread", "forum-post"]) {
  for (const invited of [false, true]) {
    test(`${keyPrefix} failed transport A-B-A without new intent restores exact payload (${invited ? "invited" : "ordinary"})`, async () => {
      const s = await setup({ keyPrefix });
      s.edit(invited ? TEXT : "original A body", invited ? REFS : []);
      s.attach();
      const transport = deferred();
      s.transport = transport;
      await s.submit();
      if (invited) await s.invite();
      assert.equal(s.text, "");
      s.navigate("b");
      s.edit("B stays", []);
      s.navigate("a");
      assert.equal(s.text, "");
      await s.finish(transport, new Error("relay unavailable"));
      const text = invited ? TEXT : "original A body";
      assert.equal(s.text, text);
      assert.deepEqual(Array.from(s.refs), invited ? REFS : []);
      assert.deepEqual(Array.from(s.imeta), MEDIA);
      const saved = draftStore.loadDraftEntry(`${keyPrefix}:a`);
      assert.equal(saved.content, text);
      assert.deepEqual(Array.from(saved.mentionRefs), invited ? REFS : []);
      assert.deepEqual(Array.from(saved.pendingImeta), MEDIA);
      assert.equal(saved.channelId, "forum");
      assert.equal(s.calls.filter((c) => c[0] === "send").length, 1);
      s.navigate("b");
      assert.equal(s.text, "B stays");
      s.navigate("a");
      assert.equal(s.text, text);
      assert.deepEqual(Array.from(s.imeta), MEDIA);
    });
  }
}

for (const intent of [
  "text-clear",
  "attachment",
  "attachment-remove",
  "delete",
  "refs",
  "new-send",
  "scope-reset",
]) {
  test(`late failed transport cannot overwrite newer ${intent}`, async () => {
    const s = await setup();
    s.edit();
    s.attach();
    const old = deferred();
    s.transport = old;
    await s.submit();
    await s.invite();
    s.navigate("b");
    s.navigate("a");
    if (intent === "text-clear") {
      s.edit("replacement", []);
      s.edit("", []);
    }
    if (intent === "attachment" || intent === "attachment-remove") {
      s.attach([{ ...MEDIA[0], url: "https://media.example/new.pdf" }]);
      if (intent === "attachment-remove") s.attach([]);
    }
    if (intent === "delete") s.deleteDraft();
    if (intent === "refs")
      s.edit(TEXT, [{ ...REFS[0], pubkey: "c".repeat(64) }]);
    let current;
    if (intent === "new-send") {
      s.edit("next send", []);
      current = deferred();
      s.transport = current;
      await s.submit();
    }
    if (intent === "scope-reset") {
      draftStore.initDraftStore("other", "wss://other.test");
      draftStore.initDraftStore("forum-test", "wss://forum.test");
    }
    const text = s.text;
    const refs = [...s.refs];
    const imeta = [...s.imeta];
    const stored = draftStore.loadDraftEntry("thread:a");
    await s.finish(old, new Error("old transport failed"));
    assert.equal(s.text, text);
    assert.deepEqual(Array.from(s.refs), refs);
    assert.deepEqual(Array.from(s.imeta), imeta);
    assert.deepEqual(draftStore.loadDraftEntry("thread:a"), stored);
    if (current) {
      await s.submit(); // The older finally cannot release this attempt.
      assert.equal(s.calls.filter((c) => c[0] === "send").length, 2);
      await s.finish(current, new Error("new transport failed"));
      assert.equal(s.text, "next send");
    }
  });
}

for (const result of ["failure", "success"]) {
  test(`off-source transport ${result} never writes B or replays publication`, async () => {
    const s = await setup();
    s.edit();
    s.attach();
    const transport = deferred();
    s.transport = transport;
    await s.submit();
    await s.invite();
    s.navigate("b");
    s.edit("B untouched", []);
    await s.finish(
      transport,
      result === "failure" ? new Error("offline") : undefined,
    );
    assert.equal(s.text, "B untouched");
    s.navigate("a");
    assert.equal(s.text, result === "failure" ? TEXT : "");
    assert.deepEqual(Array.from(s.imeta), result === "failure" ? MEDIA : []);
    assert.equal(s.calls.filter((c) => c[0] === "send").length, 1);
    assert.equal(s.calls.filter((c) => c[0] === "add").length, 1);
  });
}

for (const action of ["navigation", "return", "edit", "unmount"]) {
  test(`clipboard settlement after ${action} cannot prepare another forum draft`, async () => {
    const s = await setup();
    s.edit();
    const gate = deferred();
    s.control.settle = gate;
    await s.submit();
    if (action === "navigation" || action === "return") {
      s.navigate("b");
      s.edit("B draft", []);
      if (action === "return") s.navigate("a");
    }
    if (action === "edit") s.edit("new authored draft", []);
    if (action === "unmount") s.unmount();
    await s.finish(gate);
    assert.equal(s.calls.length, 0);
    assert.equal(s.prompt.open, false);
    if (action === "navigation") assert.equal(s.text, "B draft");
    if (action === "return") assert.equal(s.text, TEXT);
    if (action === "edit") assert.equal(s.text, "new authored draft");
  });
}

for (const change of [
  "none",
  "link",
  "team",
  "membership",
  "inactive",
  "removed",
]) {
  test(`mounted forum displayed team rejects fresh ${change} during deferred selection`, async () => {
    const s = await setup({ realMentions: true });
    await s.open();
    const gate = deferred();
    s.control.selection = gate;
    await s.choose("Crew");
    assert.ok(s.calls.some(([name]) => name === "selection-prepare"));
    if (change === "link")
      s.control.agents = [{ ...s.control.agents[0], personaId: "other" }];
    if (change === "team") s.control.teams = [];
    if (change === "membership") {
      s.control.personas = [
        ...s.control.personas,
        { id: "other", displayName: "Other", isActive: true },
      ];
      s.control.teams = [{ ...s.control.teams[0], personaIds: ["other"] }];
    }
    if (change === "inactive")
      s.control.personas = [{ ...s.control.personas[0], isActive: false }];
    if (change === "removed") s.control.personas = [];
    await s.finish(gate);
    assert.equal(s.text, change === "none" ? "Crew(@Scout) " : "");
    assert.deepEqual(
      s.mentions.extractMentionPubkeys(s.text),
      change === "none" ? [KEY] : [],
    );
  });
}

test("mounted main composer uses the displayed identity at actual commit", async () => {
  const s = await setup({ realMentions: true, main: true });
  await s.open();
  await s.choose("Scout");
  assert.equal(s.text, "@Scout ");
  assert.deepEqual(s.mentions.extractMentionPubkeys(s.text), [KEY]);
});

for (const main of [false, true]) {
  for (const departure of [
    "edit-restore",
    "silent-restore",
    "caret-return",
    "visit-return",
    "blur-return",
    "unmount",
  ]) {
    test(`mounted ${main ? "main" : "forum"} pending choice retires on ${departure}`, async () => {
      const { act } = await import("@testing-library/react");
      const s = await setup({ realMentions: true, main });
      act(() => s.richText.editor.commands.insertContent("hello"));
      const original = s.richText;
      const position = original.getPlainTextAndCursor();
      await s.open();
      const gate = deferred();
      s.control.selection = gate;
      await s.choose("Scout");
      assert.ok(s.calls.some(([name]) => name === "selection-prepare"));
      if (departure === "edit-restore")
        act(() => {
          original.editor.commands.insertContent("x");
          original.editor.commands.deleteRange({ from: 6, to: 7 });
        });
      if (departure === "silent-restore")
        act(() => {
          original.editor.commands.setContent("different", {
            emitUpdate: false,
          });
          original.editor.commands.setContent("hello", { emitUpdate: false });
        });
      if (departure === "caret-return")
        act(() => {
          original.editor.commands.setTextSelection(1);
          original.editor.commands.setTextSelection(6);
        });
      if (departure === "visit-return") {
        s.navigate("b");
        s.navigate("a");
      }
      if (departure === "blur-return")
        act(() => {
          original.editor.view.dom.focus();
          const outside = document.createElement("button");
          document.body.append(outside);
          outside.focus();
          original.editor.view.dom.focus();
          outside.remove();
        });
      if (departure === "unmount") s.unmount();
      await s.finish(gate);
      assert.equal(s.text, position.text, "late preparation must not insert");
      assert.deepEqual(s.mentions.extractMentionPubkeys(s.text), []);
    });
  }
}

for (const main of [false, true]) {
  test(`mounted ${main ? "main" : "forum"} keyboard insertion settles the literal label caret`, async () => {
    const { act, fireEvent } = await import("@testing-library/react");
    const s = await setup({ realMentions: true, main });
    await s.open();
    // The first displayed row owns Enter, through the actual composer handler.
    const first = s.mentions.suggestions[0];
    act(() => fireEvent.keyDown(s.richText.editor.view.dom, { key: "Enter" }));
    await act(async () => {});
    assert.ok(
      s.text.includes(first.displayName),
      JSON.stringify({
        first,
        text: s.text,
        calls: s.calls,
        open: s.mentions.isMentionOpen,
      }),
    );
    const before = s.text;
    act(() => s.richText.editor.commands.insertContent("next"));
    assert.equal(s.text, `${before}next`);
    assert.deepEqual(s.mentions.extractMentionPubkeys(s.text), [KEY]);
  });
}

for (const change of ["link", "replacement", "inactive", "removed", "rename"]) {
  test(`mounted main persona identity binds fresh ${change} without retargeting`, async () => {
    const s = await setup({ realMentions: true, main: true });
    await s.open();
    const gate = deferred();
    s.control.selection = gate;
    await s.choose("Scout");
    assert.ok(s.calls.some(([name]) => name === "selection-prepare"));
    if (change === "link")
      s.control.agents = [{ ...s.control.agents[0], personaId: "other" }];
    if (change === "replacement")
      s.control.agents = [{ ...s.control.agents[0], pubkey: "c".repeat(64) }];
    if (change === "inactive")
      s.control.personas = [{ ...s.control.personas[0], isActive: false }];
    if (change === "removed") s.control.personas = [];
    if (change === "rename")
      s.control.agents = [{ ...s.control.agents[0], name: "Renamed" }];
    await s.finish(gate);
    assert.equal(s.text, change === "rename" ? "@Scout " : "");
    assert.deepEqual(
      s.mentions.extractMentionPubkeys(s.text),
      change === "rename" ? [KEY] : [],
    );
  });
}

for (const changed of [false, true]) {
  test(`mounted main Options focus and pointer pin ${changed ? "reject changed linkage" : "preserve displayed intent"}`, async () => {
    const { act, fireEvent } = await import("@testing-library/react");
    const s = await setup({ realMentions: true, main: true });
    act(() => s.richText.editor.view.dom.focus());
    await s.open();
    act(() =>
      fireEvent.keyDown(s.richText.editor.view.dom, {
        key: "Tab",
        shiftKey: true,
      }),
    );
    const options = s.view.getByRole("switch", {
      name: "Automatically mention agents",
    });
    assert.equal(document.activeElement, options);
    assert.equal(s.mentions.isMentionOpen, true);
    const pin = s.view.getByRole("button", {
      name: "Automatically mention Scout",
      exact: true,
    });
    const gate = deferred();
    s.control.selection = gate;
    act(() => fireEvent.mouseDown(pin));
    act(() => fireEvent.click(pin));
    await act(async () => {});
    assert.equal(s.text, "");
    if (changed)
      s.control.agents = [{ ...s.control.agents[0], personaId: "other" }];
    await s.finish(gate);
    assert.equal(s.text, changed ? "" : "@Scout ");
    assert.ok(
      s.view.getByRole("button", {
        name: changed
          ? "Automatically mention Scout"
          : "Don't automatically mention Scout in this thread",
        exact: true,
      }),
    );
  });
}

for (const active of [false, true]) {
  test(`mounted standalone persona selection checks its deferred active definition: ${active}`, async () => {
    const s = await setup({ realMentions: true });
    s.control.agents = [];
    s.navigate("b");
    await s.open();
    assert.equal(s.mentions.suggestions[0].kind, "persona");
    const gate = deferred();
    s.control.definition = gate;
    await s.choose("Scout");
    s.control.personas = [{ ...s.control.personas[0], isActive: active }];
    await s.finish(gate);
    assert.equal(s.text, active ? "@Scout " : "");
    assert.equal(
      s.mentions.extractMentionPersonas(s.text).length,
      active ? 1 : 0,
    );
  });
}
