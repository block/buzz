import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

/**
 * The paste-side trust gate, driven through the hook the composers pass to
 * `handleMentionClipboardPaste`.
 *
 * Clipboard HTML is attacker-authored, so a `label → pubkey` pair being
 * *visible* in a paste proves only that the user saw a name. These cases pin
 * the other half: the pair has to be one this community's own state names.
 */

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** 64-hex, the only shape `parseMentionClipboardRecords` lets through. */
const JOHN = "a".repeat(64);
const IMPOSTOR = "b".repeat(64);
const FIZZ = "c".repeat(64);

const johnSmith = { label: "John Smith", pubkey: JOHN, isAgent: false };

/** Commands the hook sent to the backend during one case. */
let invocations = [];
/** `get_users_batch` responses, keyed by lowercase pubkey. */
let relayProfiles = new Map();
/** Every client a case rendered, so its cache timers die with the case. */
const queryClients = [];

before(() => {
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      invocations.push({ command, args });
      if (command !== "get_users_batch") {
        throw new Error(`Unexpected command in this test: ${command}`);
      }
      const profiles = {};
      const missing = [];
      for (const pubkey of args.pubkeys) {
        const profile = relayProfiles.get(pubkey);
        if (profile) profiles[pubkey] = profile;
        else missing.push(pubkey);
      }
      return { profiles, missing };
    },
  };
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    localStorage: dom.window.localStorage,
    window: dom.window,
  });
});

afterEach(async () => {
  invocations = [];
  relayProfiles = new Map();
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  // Cached entries schedule garbage-collection timers; left running they hold
  // the test runner's event loop open long after the assertions pass.
  for (const client of queryClients.splice(0)) client.clear();
});

after(() => dom.window.close());

/**
 * Render the hook with the trusted state a composer would hand it.
 *
 * The QueryClient is per-case so a cached profile entry from one case cannot
 * silently vouch for the next.
 */
async function renderVerifier({ mentionCandidates = [], profiles } = {}) {
  const React = await import("react");
  const { renderHook } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { useVerifyMentionIdentities } = await import(
    "./useVerifyMentionIdentities.ts"
  );

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClients.push(queryClient);
  const { result } = renderHook(
    () => useVerifyMentionIdentities({ mentionCandidates, profiles }),
    {
      wrapper: ({ children }) =>
        React.createElement(
          QueryClientProvider,
          { client: queryClient },
          children,
        ),
    },
  );
  return { queryClient, verify: result.current };
}

function relayCalls() {
  return invocations.filter((entry) => entry.command === "get_users_batch");
}

test("vouches for a pair the surface's own profile lookup names", async () => {
  const { verify } = await renderVerifier({
    profiles: { [JOHN]: { displayName: "John Smith" } },
  });

  assert.deepEqual(await verify([johnSmith]), [johnSmith]);
  assert.deepEqual(relayCalls(), [], "local state must not cost a round trip");
});

test("vouches for any alias the renderer could have chipped", async () => {
  // `collectProfileAliases` is what turned a `p` tag into the label a copy
  // carries; the check has to accept the same set or a legitimate copy of a
  // kind-0-`name` or NIP-05 chip fails closed.
  const { verify } = await renderVerifier({
    profiles: {
      [JOHN]: {
        displayName: "J. Smith",
        name: "John Smith",
        nip05Handle: "jsmith@example.com",
      },
    },
  });

  assert.deepEqual(await verify([johnSmith]), [johnSmith]);
  assert.deepEqual(
    await verify([{ label: "jsmith", pubkey: JOHN, isAgent: false }]),
    [{ label: "jsmith", pubkey: JOHN, isAgent: false }],
  );
});

test("vouches for the persona name a managed agent's chip renders", async () => {
  // An agent renders under its persona, which is no alias of its kind-0
  // profile — the directory that named it has to be able to answer.
  const fizz = { label: "Fizz", pubkey: FIZZ, isAgent: true };
  const { verify } = await renderVerifier({
    mentionCandidates: [
      {
        kind: "identity",
        pubkey: FIZZ.toUpperCase(),
        displayName: "Fizz",
        isMember: true,
        isAgent: true,
      },
    ],
  });

  assert.deepEqual(await verify([fizz]), [fizz]);
  assert.deepEqual(relayCalls(), []);
});

test("asks the relay about a pubkey no local state has seen", async () => {
  // The headline case: a mention of someone who is not a member of the
  // channel being pasted into, so nothing local can speak for the pair.
  relayProfiles.set(JOHN, {
    pubkey: JOHN,
    display_name: "John Smith",
    is_agent: false,
  });
  const { verify } = await renderVerifier();

  assert.deepEqual(await verify([johnSmith]), [johnSmith]);
  assert.deepEqual(relayCalls().length, 1);
});

test("drops a visible pair this community has never seen", async () => {
  // The hostile shape: `<span data-mention-pubkey="<their key>"
  // data-mention-label="John Smith">@John Smith</span>`. It pastes plausibly
  // and the user sees it — and no trusted state names that key "John Smith".
  const forged = { label: "John Smith", pubkey: IMPOSTOR, isAgent: false };
  const { verify } = await renderVerifier({
    profiles: { [JOHN]: { displayName: "John Smith" } },
  });

  assert.deepEqual(await verify([forged]), []);
});

test("drops a pair whose key the relay knows under another name", async () => {
  // Same forgery against a key that does exist here. Knowing the key is not
  // the question; the pair being one this community holds is.
  relayProfiles.set(IMPOSTOR, {
    pubkey: IMPOSTOR,
    display_name: "Mallory",
    is_agent: false,
  });
  const { verify } = await renderVerifier();

  assert.deepEqual(
    await verify([{ label: "John Smith", pubkey: IMPOSTOR, isAgent: false }]),
    [],
  );
});

test("separates the vouched-for from the forged in one paste", async () => {
  relayProfiles.set(JOHN, {
    pubkey: JOHN,
    display_name: "John Smith",
    is_agent: false,
  });
  const forged = { label: "Fizz", pubkey: IMPOSTOR, isAgent: true };
  const { verify } = await renderVerifier();

  assert.deepEqual(await verify([johnSmith, forged]), [johnSmith]);
  // Both unknown pubkeys resolved in one request.
  assert.deepEqual(relayCalls().length, 1);
  assert.deepEqual(
    relayCalls()[0].args.pubkeys.sort(),
    [JOHN, IMPOSTOR].sort(),
  );
});

test("answers from a fresh users-batch entry without a round trip", async () => {
  const { queryClient, verify } = await renderVerifier();
  const { usersBatchEntryKey } = await import("@/features/profile/hooks.ts");
  queryClient.setQueryData(usersBatchEntryKey(JOHN), {
    summary: { displayName: "John Smith" },
    fetchedAt: Date.now(),
  });

  assert.deepEqual(await verify([johnSmith]), [johnSmith]);
  assert.deepEqual(relayCalls(), []);
});

test("refetches rather than trusting an entry the profile hooks call stale", async () => {
  relayProfiles.set(JOHN, {
    pubkey: JOHN,
    display_name: "John Smith",
    is_agent: false,
  });
  const { queryClient, verify } = await renderVerifier();
  const { usersBatchEntryKey, USERS_BATCH_ENTRY_FRESH_MS } = await import(
    "@/features/profile/hooks.ts"
  );
  // A stale entry recording a relay-confirmed miss: believing it would drop a
  // legitimate identity whose profile has since arrived.
  queryClient.setQueryData(usersBatchEntryKey(JOHN), {
    summary: null,
    fetchedAt: Date.now() - USERS_BATCH_ENTRY_FRESH_MS - 1,
  });

  assert.deepEqual(await verify([johnSmith]), [johnSmith]);
  assert.deepEqual(relayCalls().length, 1);
});

test("tolerates what a pasteboard leaves on a declared label", async () => {
  // A round trip through another app can swap spaces for U+00A0 and pad the
  // attribute; the pair is still the one the community holds.
  const roundTripped = {
    label: ` John${String.fromCharCode(0xa0)}smith `,
    pubkey: JOHN,
    isAgent: false,
  };
  const { verify } = await renderVerifier({
    profiles: { [JOHN]: { displayName: "John Smith" } },
  });

  assert.deepEqual(await verify([roundTripped]), [roundTripped]);
});

test("drops a pair with no label at all", async () => {
  const { verify } = await renderVerifier({
    profiles: { [JOHN]: { displayName: "" } },
  });

  assert.deepEqual(
    await verify([{ label: "", pubkey: JOHN, isAgent: false }]),
    [],
  );
});
