/**
 * Rule 1 regression: the persona card's MAIN click records a PERSONA target,
 * never an explicit pubkey — even during the archive-snapshot fail-open window,
 * when pickProfileAgent transiently selects an archived sibling.
 *
 * Why a mounted render test rather than a pure resolver test:
 *   resolveCanonicalManagedAgent (unit-tested separately) proves a persona
 *   target self-corrects to the live sibling after hydration — but it assumes
 *   the card emits a persona target. The defect being closed is the card
 *   emitting a durable *pubkey* target that survives hydration. Only mounting
 *   the real card and firing its main click catches a mutation that reverts
 *   onClick back to onOpenAgentProfile(agent.pubkey). AgentPersonaCard is
 *   module-local, so the whole section is mounted.
 *
 * Fail-open is reproduced faithfully: the list_archived_identities IPC call
 * never settles, so useIsArchivedPredicate returns all-live at click time and
 * pickProfileAgent selects the archived-first sibling — exactly the transient
 * window the durable pubkey target used to strand the panel on.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// Track every client so afterEach can drop cached queries. A query left pending
// (the fail-open archive snapshot) plus react-query's default gcTime schedules
// timers that outlive the test and stall the shared `pnpm test` process.
const clients = [];

let act;
let cleanup;
let fireEvent;
let render;
let screen;
let createElement;
let QueryClient;
let QueryClientProvider;
let UnifiedAgentsSection;

const ipcHandlers = new Map();

const SELF_PK = "c".repeat(64);
const ARCHIVED_PK = "a".repeat(64);
const LIVE_PK = "b".repeat(64);

function agent(overrides = {}) {
  return {
    pubkey: LIVE_PK,
    name: "Instance",
    personaId: "persona-1",
    status: "stopped",
    model: null,
    modelSource: "global",
    lastError: null,
    lastErrorCode: null,
    needsRestart: false,
    personaOrphaned: false,
    ...overrides,
  };
}

function persona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Fizz Prime",
    avatarUrl: null,
    model: null,
    isBuiltIn: false,
    sourceTeam: null,
    ...overrides,
  };
}

function baseProps(overrides = {}) {
  return {
    defaultModel: "gpt-x",
    actionErrorMessage: null,
    actionNoticeMessage: null,
    agents: [],
    agentsError: null,
    isActionPending: false,
    isAgentsLoading: false,
    restartingAgentPubkey: null,
    startingAgentPubkey: null,
    startingPersonaIds: new Set(),
    onOpenAgentProfile: () => {},
    onOpenPersonaProfile: () => {},
    onRestartAgent: () => {},
    onStartAgent: () => {},
    onStartPersona: () => {},
    personas: [],
    personasError: null,
    personaFeedbackErrorMessage: null,
    personaFeedbackNoticeMessage: null,
    isPersonasLoading: false,
    isPersonasPending: false,
    onOpenCatalog: () => {},
    onDuplicatePersona: () => {},
    onEditPersona: () => {},
    onSharePersona: () => {},
    onDeactivatePersona: () => {},
    onDeletePersona: () => {},
    ...overrides,
  };
}

function renderSection(props) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  clients.push(client);
  return render(
    createElement(
      QueryClientProvider,
      { client },
      createElement(UnifiedAgentsSection, props),
    ),
  );
}

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    window: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.matchMedia = () => ({
    matches: true,
    addEventListener() {},
    removeEventListener() {},
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      const handler = ipcHandlers.get(cmd);
      if (handler) return handler(args);
      return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
    },
    transformCallback: () => Math.random(),
  };

  ({ act, cleanup, fireEvent, render, screen } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ UnifiedAgentsSection } = await import("./UnifiedAgentsSection.tsx"));
});

afterEach(() => {
  cleanup?.();
  for (const client of clients.splice(0)) {
    client.cancelQueries();
    client.clear();
  }
  ipcHandlers.clear();
});

after(() => dom.window.close());

function installFailOpenIpc() {
  ipcHandlers.set("get_identity", () =>
    Promise.resolve({ pubkey: SELF_PK, display_name: "Me" }),
  );
  // Never resolves: the archive snapshot stays loading, so the predicate is
  // fail-open (treats every identity as live) for the whole test.
  ipcHandlers.set("list_archived_identities", () => new Promise(() => {}));
  ipcHandlers.set("get_user_profile", () =>
    Promise.resolve({
      pubkey: LIVE_PK,
      display_name: null,
      avatar_url: null,
      about: null,
      nip05_handle: null,
      owner_pubkey: null,
    }),
  );
}

test("persona card main click records a persona target, never an explicit pubkey", async () => {
  installFailOpenIpc();

  let recordedPersona;
  const onOpenAgentProfile = () => {
    throw new Error("card main click must not open an explicit pubkey target");
  };
  const onOpenPersonaProfile = (persona) => {
    recordedPersona = persona;
  };

  // Archived sibling sorts first by name, so under fail-open pickProfileAgent
  // selects it — the card displays the archived identity at click time. A
  // durable pubkey target would strand the panel there after hydration.
  const agents = [
    agent({ pubkey: ARCHIVED_PK, name: "Archived Sibling" }),
    agent({ pubkey: LIVE_PK, name: "Zed Sibling" }),
  ];

  await act(async () => {
    renderSection(
      baseProps({
        agents,
        personas: [persona()],
        onOpenAgentProfile,
        onOpenPersonaProfile,
      }),
    );
  });

  fireEvent.click(
    screen.getByRole("button", { name: "Fizz Prime agent profile" }),
  );

  assert.ok(recordedPersona, "the click must record a persona target");
  assert.equal(recordedPersona.id, "persona-1");
});

test("persona card main click records a persona target even for a stopped errored agent", async () => {
  installFailOpenIpc();

  let recordedPersona;
  await act(async () => {
    renderSection(
      baseProps({
        agents: [
          agent({
            pubkey: LIVE_PK,
            name: "Errored",
            status: "stopped",
            lastError: "boom",
          }),
        ],
        personas: [persona()],
        onOpenAgentProfile: () => {
          throw new Error("main click must not open an explicit pubkey target");
        },
        onOpenPersonaProfile: (persona) => {
          recordedPersona = persona;
        },
      }),
    );
  });

  fireEvent.click(
    screen.getByRole("button", { name: "Fizz Prime agent profile" }),
  );

  assert.equal(recordedPersona?.id, "persona-1");
});

test("errored avatar affordance still opens the explicit pubkey on the runtime tab", async () => {
  installFailOpenIpc();

  const opened = [];
  await act(async () => {
    renderSection(
      baseProps({
        agents: [
          agent({
            pubkey: LIVE_PK,
            name: "Errored",
            status: "stopped",
            lastError: "boom",
          }),
        ],
        personas: [persona()],
        onOpenAgentProfile: (pubkey, options) => {
          opened.push({ pubkey, options });
        },
        onOpenPersonaProfile: () => {
          throw new Error("the error affordance must open the explicit pubkey");
        },
      }),
    );
  });

  // The error badge is the deliberate explicit-pubkey path preserved for
  // manage/diagnose access; it is the reserved instance/error navigation that
  // rule 1 keeps valid, unchanged by the main-click fix.
  fireEvent.click(screen.getByTestId(`agent-runtime-error-${LIVE_PK}`));

  assert.deepEqual(opened, [{ pubkey: LIVE_PK, options: { tab: "runtime" } }]);
});
