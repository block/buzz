/**
 * Consumer regression for the huddle participant avatar fallback initials
 * (Astra P2 on PR-D2).
 *
 * `buildParticipantIdentities` renders unnamed participants with generated
 * role-prefixed key labels ("Participant npub1…", "Agent npub1…"). `getInitials`
 * only recognizes a whole compact/full npub as a key label, so before the
 * `initialsLabel` fix every one of those labels collapsed onto the word
 * initials "PN"/"AN" — losing the distinct avatar initials that raw hex
 * keys used to produce (PA/PB, AA/AB). This test mounts the real
 * `HuddleParticipantsControl` (room appearance) with the real
 * `CommunitiesProvider`/`useUsersBatchQuery` wiring, exactly as the huddle
 * room renders it, so it proves the wiring — not just the helper.
 *
 * Fixture design notes:
 * - The self profile is a DISTINCT key ("c"×64) from the unnamed identities:
 *   a same-key self profile would replace the unnamed participant it is
 *   meant to leave alone, and then the test would no longer assert a
 *   genuinely rendered unnamed fallback.
 * - The named agent comes through the real relay-agents query cache, so the
 *   authored-name branch of `buildParticipantIdentities` runs its production
 *   path (profiles lookup first, agent name second, key label last).
 */

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { JSDOM } from "jsdom";

// HuddleParticipantsControl reads communities through the real
// CommunitiesProvider, whose storage helpers touch `window.localStorage`
// (fail-safe in the app, fatal in bare Node). JSDOM supplies a real one.
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  document: dom.window.document,
  localStorage: dom.window.localStorage,
  self: dom.window,
  window: dom.window,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
  writable: true,
});
dom.window.matchMedia = () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});
globalThis.matchMedia = dom.window.matchMedia;
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
dom.window.cancelAnimationFrame = (id) => clearTimeout(id);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame;

// Bulk-copy the DOM constructors Radix references without a window prefix,
// so any component touched during static rendering finds the same globals
// it would in the app window.
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (
    !(key in globalThis) &&
    (key.startsWith("HTML") ||
      key.startsWith("SVG") ||
      [
        "Element",
        "Node",
        "NodeFilter",
        "NodeList",
        "NamedNodeMap",
        "Event",
        "CustomEvent",
        "MouseEvent",
        "KeyboardEvent",
        "FocusEvent",
        "InputEvent",
        "PointerEvent",
        "Text",
        "Comment",
        "DocumentFragment",
        "Range",
        "Selection",
      ].includes(key))
  ) {
    const value = dom.window[key];
    if (value !== undefined) globalThis[key] = value;
  }
}

function seedCommunity() {
  window.localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      {
        id: "community-a",
        name: "Community A",
        relayUrl: "wss://relay.example",
        pubkey: "1".repeat(64),
        addedAt: "2026-01-01T00:00:00Z",
      },
    ]),
  );
  window.localStorage.setItem("buzz-active-community-id", "community-a");
}

const UNNAMED_PARTICIPANT = "a".repeat(64); // npub1424…rcaj → initials RC
const UNNAMED_AGENT = "b".repeat(64); // npub1hwa…04hu → initials 04
const NAMED_SELF = "c".repeat(64);
const NAMED_AGENT = "d".repeat(64); // npub1mhw…dmpv → key initials DM if wrong

let React;
let renderToStaticMarkup;
let QueryClient;
let QueryClientProvider;
let CommunitiesProvider;
let TooltipProvider;
let HuddleParticipantsControl;
let relayAgentsQueryKey;

before(async () => {
  ({ default: React } = await import("react"));
  ({ renderToStaticMarkup } = await import("react-dom/server"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip.tsx"));
  ({ HuddleParticipantsControl } = await import("./ParticipantList.tsx"));
  ({ relayAgentsQueryKey } = await import("@/features/agents/hooks.ts"));
});

after(() => dom.window.close());

function renderRoom() {
  seedCommunity();
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  // Named agent through the real relay-agents query the control mounts.
  client.setQueryData(relayAgentsQueryKey, [
    { name: "Helpful Researcher", pubkey: NAMED_AGENT },
  ]);
  let html;
  try {
    html = renderToStaticMarkup(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(
            TooltipProvider,
            null,
            React.createElement(HuddleParticipantsControl, {
              appearance: "room",
              agentPubkeys: [UNNAMED_AGENT, NAMED_AGENT],
              participants: [
                UNNAMED_PARTICIPANT,
                UNNAMED_AGENT,
                NAMED_SELF,
                NAMED_AGENT,
              ],
              selfProfile: {
                avatarUrl: null,
                displayName: "Alice Example",
                pubkey: NAMED_SELF,
              },
            }),
          ),
        ),
      ),
    );
  } finally {
    // Drop the query cache (and its gc timers) and close the window so the
    // test process does not wait on their handles after the assertions.
    client.clear();
  }
  return html;
}

test("unnamed participants and agents keep distinct key-tail initials", () => {
  const html = renderRoom();

  assert.match(html, /Participant npub1424…rcaj/);
  assert.match(html, />RC</);
  assert.match(html, /Agent npub1hwa…04hu/);
  assert.match(html, />04</);
  // The pre-fix regression: every generated "Participant/Agent npub1…"
  // label collapsed onto the same word initials.
  assert.doesNotMatch(html, />(PN|AN)</);
});

test("authored names — self profile and relay agent — keep name initials", () => {
  const html = renderRoom();

  assert.match(html, /Alice Example/);
  assert.match(html, />AE</);
  assert.match(html, /Helpful Researcher/);
  assert.match(html, />HR</);
  // The named agent must not fall back to its key-tail initials.
  assert.doesNotMatch(html, />DM</);
});
