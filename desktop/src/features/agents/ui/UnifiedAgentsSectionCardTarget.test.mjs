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
let useAgentAvailabilityLookup;

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

function Surface(props) {
  const { getAvailability } = useAgentAvailabilityLookup(
    props.agents.map((a) => a.pubkey),
  );
  return createElement(UnifiedAgentsSection, { ...props, getAvailability });
}

function renderSection(props) {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { gcTime: 0 },
    },
  });
  clients.push(client);
  return render(
    createElement(
      QueryClientProvider,
      { client },
      createElement(Surface, props),
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
  ({ useAgentAvailabilityLookup } = await import(
    "../lib/useAgentAvailability.ts"
  ));
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

for (const kind of ["persona", "custom", "unknown"]) {
  test(`${kind} stopped card uses exact-key presence without inventing lifecycle controls`, async () => {
    installFailOpenIpc();
    const { relayClient } = await import("../../../shared/api/relayClient.ts");
    const originalConnection = relayClient.getConnectionState;
    const originalSubscribe = relayClient.subscribeToConnectionState;
    relayClient.getConnectionState = () => "connected";
    relayClient.subscribeToConnectionState = () => () => {};
    let snapshot = { [ARCHIVED_PK]: "online" };
    ipcHandlers.set("get_presence", () => Promise.resolve(snapshot));
    const starts = [];
    const props = baseProps({
      agents: [
        agent({
          personaId:
            kind === "custom"
              ? null
              : kind === "unknown"
                ? "missing"
                : "persona-1",
        }),
      ],
      personas: kind === "persona" ? [persona()] : [],
      onStartAgent: (key) => starts.push(key),
      onRestartAgent: () => {
        throw new Error("presence must not cause Restart");
      },
    });
    try {
      await act(async () => renderSection(props));
      const client = clients.at(-1);
      const refresh = async () => {
        await act(async () => {
          await client.invalidateQueries({ queryKey: ["presence"] });
        });
        await act(async () => {
          await new Promise((resolve) => setTimeout(resolve, 10));
        });
      };
      await refresh();
      // Different-key Online must not suppress this identity's ordinary Start.
      fireEvent.click(screen.getByTestId(`agent-runtime-start-${LIVE_PK}`));
      assert.deepEqual(starts, [LIVE_PK]);
      starts.length = 0;
      for (const status of ["online", "away", "offline", "online"]) {
        snapshot = { [LIVE_PK]: status };
        await refresh();
        const start = screen.queryByTestId(`agent-runtime-start-${LIVE_PK}`);
        if (status === "offline") {
          assert.ok(start);
          fireEvent.click(start);
          assert.deepEqual(starts, [LIVE_PK]);
          starts.length = 0;
        } else {
          assert.equal(
            Boolean(start),
            false,
            "active exact-key presence must remove Start",
          );
          const dot = screen.getByTestId(`agent-runtime-active-${LIVE_PK}`);
          assert.match(
            dot.getAttribute("aria-label"),
            new RegExp(status === "online" ? "Online$" : "Away$"),
          );
          fireEvent.click(dot);
          fireEvent.keyDown(dot, { key: "Enter" });
          fireEvent.keyDown(dot, { key: " " });
          assert.deepEqual(starts, []);
          assert.equal(
            Boolean(screen.queryByRole("button", { name: /Stop/ })),
            false,
          );
        }
      }
      // A successful omitted entry is the existing relay expiry/missing path.
      snapshot = {};
      await refresh();
      fireEvent.click(screen.getByTestId(`agent-runtime-start-${LIVE_PK}`));
      assert.deepEqual(starts, [LIVE_PK]);
    } finally {
      relayClient.getConnectionState = originalConnection;
      relayClient.subscribeToConnectionState = originalSubscribe;
    }
  });
}

test("N cards share a snapshot, one poll, failure recovery and live subscription lifecycle", async (t) => {
  installFailOpenIpc();
  const { relayClient } = await import("../../../shared/api/relayClient.ts");
  const { usePresenceSubscription, useSetPresenceMutation } = await import(
    "../../presence/hooks.ts"
  );
  const original = {
    getConnectionState: relayClient.getConnectionState,
    subscribeToConnectionState: relayClient.subscribeToConnectionState,
    subscribeToReconnects: relayClient.subscribeToReconnects,
    subscribeLive: relayClient.subscribeLive,
    sendPresence: relayClient.sendPresence,
  };
  const subscriptions = [];
  const requests = [];
  let fail = false;
  let finishSnapshot;
  ipcHandlers.set("get_presence", ({ pubkeys }) => {
    requests.push(pubkeys);
    if (fail) return Promise.reject("relay unreachable: request timed out");
    return new Promise((resolve) => {
      finishSnapshot = resolve;
    });
  });
  relayClient.sendPresence = async () => {};
  relayClient.getConnectionState = () => "connected";
  relayClient.subscribeToConnectionState = () => () => {};
  relayClient.subscribeToReconnects = () => () => {};
  relayClient.subscribeLive = async (filter, onEvent, onReady) => {
    const sub = { filter, onEvent, closed: false };
    subscriptions.push(sub);
    onReady("eose");
    return async () => {
      sub.closed = true;
    };
  };
  Object.defineProperty(dom.window.document, "visibilityState", {
    configurable: true,
    value: "visible",
  });
  const originalFocus = dom.window.document.hasFocus;
  dom.window.document.hasFocus = () => true;
  t.mock.timers.enable({ apis: ["setInterval"] });
  let setPresence;
  function SubscribedSurface(props) {
    setPresence = useSetPresenceMutation(SELF_PK);
    usePresenceSubscription();
    return createElement(Surface, props);
  }
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { gcTime: 0 },
    },
  });
  clients.push(client);
  const props = baseProps({
    agents: [
      agent({ pubkey: LIVE_PK, status: "running" }),
      agent({ pubkey: ARCHIVED_PK, personaId: null, status: "running" }),
      agent({ pubkey: SELF_PK, personaId: "missing", status: "running" }),
    ],
    personas: [persona()],
  });
  const tree = (next) =>
    createElement(
      QueryClientProvider,
      { client },
      createElement(SubscribedSurface, next),
    );
  const settle = async () =>
    act(async () => {
      await new Promise((r) => setTimeout(r, 120));
    });
  try {
    let view;
    await act(async () => {
      view = render(tree(props));
    });
    assert.equal(
      requests.length,
      1,
      "one in-flight request for persona/custom/unknown rows",
    );
    await act(async () => {
      view.rerender(tree({ ...props, agents: [...props.agents].reverse() }));
    });
    assert.equal(
      requests.length,
      1,
      "reordering while the snapshot is pending does not refetch",
    );
    await act(async () => {
      finishSnapshot({});
    });
    await settle();
    assert.deepEqual(requests[0], [ARCHIVED_PK, LIVE_PK, SELF_PK]);
    assert.equal(subscriptions.length, 1);
    assert.deepEqual(subscriptions[0].filter.authors, requests[0]);
    assert.equal(
      client
        .getQueryCache()
        .find({ queryKey: ["presence", ...requests[0]] })
        .getObserversCount(),
      1,
    );
    await act(async () => {
      subscriptions[0].onEvent({ pubkey: LIVE_PK, content: "away" });
    });
    await settle();
    assert.match(
      screen
        .getByTestId(`agent-runtime-active-${LIVE_PK}`)
        .getAttribute("aria-label"),
      /Away$/,
    );
    assert.match(
      screen
        .getByTestId(`agent-runtime-active-${ARCHIVED_PK}`)
        .getAttribute("aria-label"),
      /Offline$/,
    );
    assert.equal(
      requests.length,
      1,
      "live exact-key update makes no snapshot requests",
    );
    fail = true;
    await act(async () => {
      t.mock.timers.tick(60000);
    });
    await settle();
    assert.equal(requests.length, 2, "one backstop poll, not one per card");
    for (const { pubkey } of props.agents) {
      assert.match(
        screen
          .getByTestId(`agent-runtime-active-${pubkey}`)
          .getAttribute("aria-label"),
        /Availability unknown$/,
      );
    }
    await act(async () => {
      subscriptions[0].onEvent({ pubkey: ARCHIVED_PK, content: "online" });
    });
    await settle();
    for (const { pubkey } of props.agents) {
      assert.match(
        screen
          .getByTestId(`agent-runtime-active-${pubkey}`)
          .getAttribute("aria-label"),
        /Availability unknown$/,
        "one live author must not resurrect a failed aggregate's cached siblings",
      );
    }
    await act(async () => {
      await setPresence.mutateAsync("online");
    });
    await settle();
    for (const { pubkey } of props.agents) {
      assert.match(
        screen
          .getByTestId(`agent-runtime-active-${pubkey}`)
          .getAttribute("aria-label"),
        /Availability unknown$/,
        "a successful self heartbeat must not heal a failed aggregate snapshot",
      );
    }
    fail = false;
    await act(async () => {
      t.mock.timers.tick(60000);
    });
    await act(async () => {
      finishSnapshot({});
    });
    await settle();
    assert.equal(requests.length, 3);
    assert.match(
      screen
        .getByTestId(`agent-runtime-active-${LIVE_PK}`)
        .getAttribute("aria-label"),
      /Offline$/,
    );
    await act(async () => {
      view.rerender(tree({ ...props, agents: [props.agents[0]] }));
    });
    await act(async () => {
      finishSnapshot({});
    });
    await settle();
    assert.deepEqual(subscriptions[1].filter.authors, [LIVE_PK]);
    assert.equal(subscriptions[0].closed, true);
    await act(async () => {
      view.unmount();
    });
    assert.equal(
      subscriptions[1].closed,
      true,
      "last surface removes its live subscription",
    );
    const count = requests.length;
    await act(async () => {
      t.mock.timers.tick(120000);
    });
    assert.equal(requests.length, count, "unmounted surfaces do not poll");
  } finally {
    t.mock.timers.reset();
    dom.window.document.hasFocus = originalFocus;
    Object.assign(relayClient, original);
  }
});
