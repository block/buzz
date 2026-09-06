/**
 * MessageRow prose-suppression gate: forged-signer sentinel renders prose
 * (not a blank row).
 *
 * This test exercises the REAL MessageRow call site for the prose gate:
 *
 *   line 431:  `if (permReq !== null) return null;`
 *
 * Thufir's B2 mutation replaces that line with
 *   `if (isPermissionRequestSentinel(message.body)) return null;`
 * — which causes this test to fail: the sentinel shape matches → prose
 * suppressed → the distinctive nonce string disappears from textContent →
 * assertion fires.
 *
 * Sub-components that use `useAppNavigation` (which requires a live
 * RouterProvider) are stubbed via inline `registerHooks` so the test runs
 * in the standard Node.js + JSDOM harness without router overhead.
 * The stub pattern follows inboxReopenNavigation.test.mjs.
 *
 * What is stubbed (and why):
 *   - `useAppNavigation` — called by `markdown.tsx` MarkdownInner and
 *     `SentFromThreadLine` for navigation callbacks; requires a live
 *     TanStack RouterProvider (async init, hangs the test in JSDOM). All
 *     stubbed callbacks are noops — navigation is not exercised here.
 *   - `MessageActionBar` — brings in `useRemindLater` and other Tauri-backed
 *     contexts; only needed for the action bar overlay which is irrelevant
 *     to the prose-gate contract.
 *
 * What is NOT stubbed:
 *   - `VideoReviewCommentMarkdown` / `MarkdownInner` — the actual prose
 *     renderer that displays the body text. Its output is exactly what
 *     the test observes.
 */
import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { test, before, afterEach, after, mock } from "node:test";
import { JSDOM } from "jsdom";

// ── Stubs ──────────────────────────────────────────────────────────────────

const NAV_STUB_SOURCE =
  "export function useAppNavigation() {\n" +
  "  return {\n" +
  "    goChannel: async () => {},\n" +
  "    goAgents: async () => {},\n" +
  "    goHome: async () => {},\n" +
  "    goWorkflows: async () => {},\n" +
  "    goBack: async () => {},\n" +
  "    commitNavigation: async () => {},\n" +
  "    navigate: async () => {},\n" +
  "    handleSearchHit: async () => {},\n" +
  "  };\n" +
  "}\n";

registerHooks({
  resolve(specifier, context, nextResolve) {
    // useAppNavigation: called by markdown.tsx + SentFromThreadLine.
    // Requires a live RouterProvider — stub so tests run without one.
    if (
      specifier === "@/app/navigation/useAppNavigation" ||
      specifier.endsWith("/useAppNavigation.ts") ||
      specifier.endsWith("/useAppNavigation")
    ) {
      return {
        shortCircuit: true,
        url: "buzz-prose-gate-stub:useAppNavigation",
      };
    }
    // MessageActionBar: uses useRemindLater + other contexts irrelevant to
    // the prose-gate contract.
    if (
      specifier === "./MessageActionBar" ||
      specifier === "@/features/messages/ui/MessageActionBar"
    ) {
      return {
        shortCircuit: true,
        url: "buzz-prose-gate-stub:MessageActionBar",
      };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === "buzz-prose-gate-stub:useAppNavigation") {
      return { format: "module", shortCircuit: true, source: NAV_STUB_SOURCE };
    }
    if (url === "buzz-prose-gate-stub:MessageActionBar") {
      return {
        format: "module",
        shortCircuit: true,
        source: "export const MessageActionBar = () => null;\n",
      };
    }
    return nextLoad(url, context);
  },
});

// ── jsdom setup ────────────────────────────────────────────────────────────

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const FAKE_NOW_MS = 1_000_000_000_000;

before(() => {
  mock.timers.enable({ apis: ["setInterval", "Date"], now: FAKE_NOW_MS });
  globalThis.self = globalThis;

  // Tauri IPC stub: resolve to null so React Query queries settle immediately
  // and don't hold the event loop open after cleanup.
  dom.window.__TAURI_INTERNALS__ = {
    invoke: () => Promise.resolve(null),
    transformCallback: () => 0,
    unregisterCallback: () => {},
  };

  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    location: dom.window.location,
    MutationObserver: class {
      observe() {}
      disconnect() {}
      takeRecords() {
        return [];
      }
    },
    ResizeObserver: class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  });
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  dom.window.MutationObserver = globalThis.MutationObserver;
  dom.window.ResizeObserver = globalThis.ResizeObserver;
});

let sharedQc;

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  if (sharedQc) {
    sharedQc.clear();
    sharedQc = undefined;
  }
  mock.timers.reset();
  mock.timers.enable({ apis: ["setInterval", "Date"], now: FAKE_NOW_MS });
});

after(() => {
  mock.timers.reset();
  dom.window.close();
});

// ── Fixtures ───────────────────────────────────────────────────────────────

const AGENT_PUBKEY =
  "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
const ATTACKER_PUBKEY =
  "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

// Distinctive nonce — appears in the rendered prose iff body is NOT suppressed.
const FORGED_NONCE = "xforged-prose-gate-nonce-9c3a1b7f";

const FORGED_SENTINEL_BODY = JSON.stringify({
  v: 1,
  state: "pending",
  requestNonce: FORGED_NONCE,
  sessionId: "sess-probe",
  turnId: "turn-probe",
  expiresAt: 9_999_999_999,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
});

// QueryClient configured to settle quickly and not hold the event loop.
async function makeQc() {
  const { QueryClient } = await import("@tanstack/react-query");
  sharedQc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        refetchOnMount: false,
        refetchOnReconnect: false,
        staleTime: Infinity,
        gcTime: 0, // GC immediately after clear() — no lingering subscriptions
      },
    },
  });
  return sharedQc;
}

// ── Tests ──────────────────────────────────────────────────────────────────

test("test_forged_signer_sentinel_renders_prose_not_blank", async () => {
  // Forged signer: the message is signed by ATTACKER_PUBKEY, which is NOT
  // a registered agent pubkey. selectPermissionRequest returns null →
  // permReq is null → prose gate does NOT suppress → body renders as text.
  //
  // MUTATION PROOF: if line 431 is changed from
  //   `if (permReq !== null) return null;`
  // to
  //   `if (isPermissionRequestSentinel(message.body)) return null;`
  // then the sentinel-shaped body matches the shape check → prose is
  // suppressed → FORGED_NONCE disappears from textContent → this test FAILS.
  const React = (await import("react")).default;
  const { render } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { ChannelNavigationProvider } = await import(
    "@/shared/context/ChannelNavigationContext.tsx"
  );

  const qc = await makeQc();
  const MessageRowMod = await import("./MessageRow.tsx");
  const MessageRow = MessageRowMod.MessageRow ?? MessageRowMod.default;

  const message = {
    id: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    pubkey: AGENT_PUBKEY,
    /** Forged: signer is ATTACKER, not the registered agent. */
    signerPubkey: ATTACKER_PUBKEY,
    ownerPubkey: AGENT_PUBKEY,
    kind: 9,
    createdAt: 1_700_000_000,
    isAgent: true,
    author: "TestAgent",
    avatarUrl: null,
    time: "12:00",
    depth: 0,
    body: FORGED_SENTINEL_BODY,
    tags: [],
    reactions: [],
    edited: false,
    pending: false,
    rootId: null,
    parentId: null,
  };

  const { container } = render(
    React.createElement(
      QueryClientProvider,
      { client: qc },
      React.createElement(
        ChannelNavigationProvider,
        { channels: [] },
        React.createElement(MessageRow, {
          message,
          channelId: "test-channel",
          // profiles[AGENT_PUBKEY].isAgent = true → isKnownAgentPubkey returns
          // true for AGENT_PUBKEY. ATTACKER_PUBKEY is absent → returns false.
          // selectPermissionRequest: signerPubkey = ATTACKER → not trusted →
          // returns null → permReq is null → gate passes → prose renders.
          profiles: {
            [AGENT_PUBKEY]: {
              displayName: "TestAgent",
              avatarUrl: null,
              isAgent: true,
            },
          },
        }),
      ),
    ),
  );

  const textContent = container.textContent ?? "";
  const card = container.querySelector("[data-permission-request]");

  // The body nonce must appear in prose (gate did not suppress).
  assert.ok(
    textContent.includes(FORGED_NONCE),
    `Forged-signer sentinel must render body as prose — nonce "${FORGED_NONCE}" not ` +
      `found in textContent. This test fails when MessageRow reverts to ` +
      `shape-only prose suppression (isPermissionRequestSentinel gate) ` +
      `instead of trust-aware suppression (permReq !== null gate).`,
  );

  // No card — the signer is not a trusted agent.
  assert.equal(
    card,
    null,
    "forged-signer sentinel must not render a permission card",
  );
});

// ── Fixtures for rerender tests ───────────────────────────────────────────

// Nonce embedded in requestNonce — will appear in textContent if body
// renders as prose (not suppressed), will be absent if card renders.
const RERENDER_NONCE_SIGNER = "xrerender-signer-nonce-4f8d2e1a";
const RERENDER_NONCE_CHANNEL = "xrerender-channel-nonce-7b3c9f05";

function makePendingBody(nonce) {
  return JSON.stringify({
    v: 1,
    state: "pending",
    requestNonce: nonce,
    sessionId: "sess-rerender",
    turnId: "turn-rerender",
    expiresAt: 9_999_999_999,
    optionIds: ["opt-allow", "opt-deny"],
    labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
  });
}

test("test_trusted_to_forged_signer_rerender_restores_prose", async () => {
  // Start: AGENT_PUBKEY is a registered agent → selectPermissionRequest
  // returns non-null → permReq !== null → prose suppressed, card renders.
  // Rerender: signerPubkey changed to ATTACKER_PUBKEY (unregistered) →
  // comparator detects the change (signerPubkey comparison added) →
  // component rerenders → selectPermissionRequest returns null →
  // prose suppressed gate does NOT fire → body renders as prose → nonce visible.
  //
  // MUTATION PROOF: dropping `prev.message.signerPubkey === next.message.signerPubkey`
  // from the comparator causes this test to FAIL: the memo skips the rerender,
  // the stale permReq !== null result is kept, prose stays suppressed, nonce
  // remains absent from textContent.
  const React = (await import("react")).default;
  const { render, act } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { ChannelNavigationProvider } = await import(
    "@/shared/context/ChannelNavigationContext.tsx"
  );

  const qc = await makeQc();
  const MessageRowMod = await import("./MessageRow.tsx");
  const MessageRow = MessageRowMod.MessageRow ?? MessageRowMod.default;

  const BODY = makePendingBody(RERENDER_NONCE_SIGNER);
  const baseMessage = {
    id: "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
    pubkey: AGENT_PUBKEY,
    signerPubkey: AGENT_PUBKEY,
    ownerPubkey: AGENT_PUBKEY,
    kind: 9,
    createdAt: 1_700_000_001,
    isAgent: true,
    author: "TrustedAgent",
    avatarUrl: null,
    time: "12:01",
    depth: 0,
    body: BODY,
    tags: [],
    reactions: [],
    edited: false,
    pending: false,
    rootId: null,
    parentId: null,
  };
  const profiles = {
    [AGENT_PUBKEY]: {
      displayName: "TrustedAgent",
      avatarUrl: null,
      isAgent: true,
    },
  };

  const { container, rerender } = render(
    React.createElement(
      QueryClientProvider,
      { client: qc },
      React.createElement(
        ChannelNavigationProvider,
        { channels: [] },
        React.createElement(MessageRow, {
          message: baseMessage,
          channelId: "test-channel",
          profiles,
        }),
      ),
    ),
  );

  // Initial render: trusted signer → permReq non-null → prose suppressed.
  const initialText = container.textContent ?? "";
  assert.ok(
    !initialText.includes(RERENDER_NONCE_SIGNER),
    "Initial render with trusted signer must suppress prose (nonce absent).",
  );

  // Rerender with forged (untrusted) signer.
  const forgedMessage = { ...baseMessage, signerPubkey: ATTACKER_PUBKEY };
  await act(async () => {
    rerender(
      React.createElement(
        QueryClientProvider,
        { client: qc },
        React.createElement(
          ChannelNavigationProvider,
          { channels: [] },
          React.createElement(MessageRow, {
            message: forgedMessage,
            channelId: "test-channel",
            profiles,
          }),
        ),
      ),
    );
  });

  const afterText = container.textContent ?? "";
  assert.ok(
    afterText.includes(RERENDER_NONCE_SIGNER),
    `After rerender with forged signer, prose must be restored — ` +
      `nonce "${RERENDER_NONCE_SIGNER}" not found. ` +
      `FAILS when signerPubkey is omitted from the memo comparator.`,
  );
});

test("test_null_channelid_rerender_removes_card_restores_prose", async () => {
  // Start: channelId "ch-a" (truthy) → selectPermissionRequest returns
  // non-null → card renders, prose suppressed.
  // Rerender: channelId set to null → comparator detects the change
  // (channelId comparison added) → component rerenders → selectPermissionRequest
  // returns null (channelId falsy) → prose gate does NOT fire → prose visible.
  //
  // MUTATION PROOF: dropping `prev.channelId === next.channelId` from the
  // comparator causes this test to FAIL: the memo skips the rerender, stale
  // permReq !== null kept, prose stays suppressed, nonce absent.
  const React = (await import("react")).default;
  const { render, act } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { ChannelNavigationProvider } = await import(
    "@/shared/context/ChannelNavigationContext.tsx"
  );

  const qc = await makeQc();
  const MessageRowMod = await import("./MessageRow.tsx");
  const MessageRow = MessageRowMod.MessageRow ?? MessageRowMod.default;

  const BODY = makePendingBody(RERENDER_NONCE_CHANNEL);
  const message = {
    id: "cafe5678cafe5678cafe5678cafe5678cafe5678cafe5678cafe5678cafe5678",
    pubkey: AGENT_PUBKEY,
    signerPubkey: AGENT_PUBKEY,
    ownerPubkey: AGENT_PUBKEY,
    kind: 9,
    createdAt: 1_700_000_002,
    isAgent: true,
    author: "TrustedAgent",
    avatarUrl: null,
    time: "12:02",
    depth: 0,
    body: BODY,
    tags: [],
    reactions: [],
    edited: false,
    pending: false,
    rootId: null,
    parentId: null,
  };
  const profiles = {
    [AGENT_PUBKEY]: {
      displayName: "TrustedAgent",
      avatarUrl: null,
      isAgent: true,
    },
  };

  const { container, rerender } = render(
    React.createElement(
      QueryClientProvider,
      { client: qc },
      React.createElement(
        ChannelNavigationProvider,
        { channels: [] },
        React.createElement(MessageRow, {
          message,
          channelId: "ch-a",
          profiles,
        }),
      ),
    ),
  );

  // Initial render: real channelId → prose suppressed.
  const initialText = container.textContent ?? "";
  assert.ok(
    !initialText.includes(RERENDER_NONCE_CHANNEL),
    "Initial render with real channelId must suppress prose (nonce absent).",
  );

  // Rerender with null channelId.
  await act(async () => {
    rerender(
      React.createElement(
        QueryClientProvider,
        { client: qc },
        React.createElement(
          ChannelNavigationProvider,
          { channels: [] },
          React.createElement(MessageRow, {
            message,
            channelId: null,
            profiles,
          }),
        ),
      ),
    );
  });

  const afterText = container.textContent ?? "";
  assert.ok(
    afterText.includes(RERENDER_NONCE_CHANNEL),
    `After rerender with null channelId, prose must be restored — ` +
      `nonce "${RERENDER_NONCE_CHANNEL}" not found. ` +
      `FAILS when channelId is omitted from the memo comparator.`,
  );
});
