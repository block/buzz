/**
 * Production wiring regression for provider community enrollment.
 *
 * The menu action names the community currently rendered by the sidebar, so
 * its IPC must carry that same relay and signer even if the workspace changes
 * before the backend executes it. This mounts the real menu and action hook,
 * clicks the real item, and inspects the Tauri boundary.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const AGENT_PUBKEY = "b".repeat(64);
const OWNER_B = "a".repeat(64);
const RELAY_B = "wss://community-b.example";
const ipcCalls = [];
const clients = [];

let act;
let cleanup;
let createElement;
let fireEvent;
let fromRawManagedAgent;
let MemberActionsMenu;
let QueryClient;
let QueryClientProvider;
let render;
let useMembersSidebarActions;
let enrollmentPromise;

function rawAgent() {
  return {
    pubkey: AGENT_PUBKEY,
    name: "Provider Agent",
    persona_id: null,
    relay_url: null,
    acp_command: "",
    agent_command: "",
    agent_args: [],
    mcp_command: "",
    turn_timeout_seconds: 320,
    idle_timeout_seconds: null,
    max_turn_duration_seconds: null,
    parallelism: 1,
    system_prompt: null,
    model: null,
    env_vars: {},
    status: "deployed",
    pid: null,
    created_at: "2026-09-09T00:00:00Z",
    updated_at: "2026-09-09T00:00:00Z",
    last_started_at: null,
    last_stopped_at: null,
    last_exit_code: null,
    last_error: null,
    last_error_code: null,
    log_path: null,
    start_on_app_launch: false,
    auto_restart_on_config_change: false,
    backend: { type: "provider", id: "remote", config: {} },
    key_custody: "provider",
    backend_agent_id: "agent-123",
    respond_to: "owner-only",
    respond_to_allowlist: [],
  };
}

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  for (const key of Object.getOwnPropertyNames(dom.window)) {
    if (key === "window" || key === "document" || key === "globalThis") {
      continue;
    }
    const value = dom.window[key];
    if (
      typeof value === "function" &&
      /^(HTML|SVG)|Element$|Event$|EventTarget$|^Node|^Document|Observer$/.test(
        key,
      )
    ) {
      globalThis[key] = value;
    }
  }
  globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  dom.window.HTMLElement.prototype.hasPointerCapture = () => false;
  dom.window.HTMLElement.prototype.releasePointerCapture = () => {};
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      if (cmd !== "start_managed_agent") {
        throw new Error(`unmocked Tauri command: ${cmd}`);
      }
      ipcCalls.push({ args, cmd });
      return rawAgent();
    },
    transformCallback: () => Math.random(),
  };

  ({ act, cleanup, fireEvent, render } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ fromRawManagedAgent } = await import("@/shared/api/tauri"));
  ({ MemberActionsMenu } = await import("./MembersSidebarMemberCard.tsx"));
  ({ useMembersSidebarActions } = await import(
    "./useMembersSidebarActions.ts"
  ));
});

afterEach(() => {
  cleanup?.();
  for (const client of clients.splice(0)) {
    client.cancelQueries();
    client.clear();
    client.unmount();
  }
  ipcCalls.length = 0;
  enrollmentPromise = undefined;
});

after(() => dom.window.close());

test("the visible community enrollment action pins relay and signer scope", async () => {
  const managedAgent = fromRawManagedAgent(rawAgent());
  const member = {
    pubkey: AGENT_PUBKEY,
    role: "bot",
    isAgent: true,
  };
  const client = new QueryClient({
    defaultOptions: {
      mutations: { gcTime: 0, retry: false },
      queries: { gcTime: 0, retry: false },
    },
  });
  clients.push(client);

  function Harness() {
    const actions = useMembersSidebarActions({
      channelId: "channel-b",
      controllableManagedBots: [managedAgent],
      currentPubkey: OWNER_B,
      getAvailability: () => null,
      onOpenChange: () => {},
      relayUrl: RELAY_B,
      removableManagedBots: [managedAgent],
    });
    return createElement(MemberActionsMenu, {
      availability: undefined,
      canChangeRole: false,
      canModerateMember: false,
      canRemoveMember: false,
      canViewActivity: false,
      disabled: actions.isActionPending,
      managedAgent,
      member,
      memberIsBot: true,
      onBan: () => {},
      onChangeRole: () => {},
      onEnrollManagedAgent: (agent) => {
        enrollmentPromise = actions.handleCommunityEnrollment(agent);
      },
      onManagedAgentAction: () => {},
      onRemoveMember: () => {},
      onTimeout: () => {},
      onUnban: () => {},
      onUntimeout: () => {},
    });
  }

  render(
    createElement(QueryClientProvider, { client }, createElement(Harness)),
  );

  const trigger = document.querySelector(
    `[data-testid="sidebar-member-menu-${AGENT_PUBKEY}"]`,
  );
  assert.ok(trigger, "the real member action menu must render");
  await act(async () => {
    fireEvent.pointerDown(
      trigger,
      new dom.window.MouseEvent("pointerdown", { bubbles: true, button: 0 }),
    );
    fireEvent.click(trigger);
  });
  const enroll = document.querySelector(
    `[data-testid="sidebar-agent-enroll-${AGENT_PUBKEY}"]`,
  );
  assert.ok(enroll, "the deployed provider agent must expose enrollment");
  await act(async () => {
    fireEvent.click(enroll);
    await enrollmentPromise;
  });

  assert.equal(ipcCalls.length, 1);
  assert.equal(ipcCalls[0].cmd, "start_managed_agent");
  assert.deepEqual(ipcCalls[0].args, {
    expectedRelayUrl: RELAY_B,
    expectedSignerPubkey: OWNER_B,
    pubkey: AGENT_PUBKEY,
    replayFloorUnix: null,
  });
});
