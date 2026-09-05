/**
 * Behavior tests for Settings → Relay admin nav reachability.
 *
 * Wes P2 round-6 finding #1: Relay admin must be independently reachable
 * without NIP-11 discovery — absent, invalid, or error discovery must not
 * hide the nav entry or redirect a direct ?section=relay-admin link away.
 *
 * Tests render the real SettingsView and assert on the sidebar DOM.
 *
 * Mutation: restoring `shouldShowRelayAdminNav(relayAdminNav)` + the
 * useModerationNavResolution hook in SettingsView.tsx hides the nav entry
 * when discovery yields "none" or stays pending, causing these tests RED.
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { SidebarProvider } from "@/shared/ui/sidebar";
import { SettingsView, settingsNavGroups } from "./SettingsView.tsx";

// ── Browser API stubs ─────────────────────────────────────────────────────────
// jsdom does not implement requestAnimationFrame or matchMedia. Stub them so
// SettingsView's `isLoaded` effect and the sidebar's responsive hook don't throw.

if (!globalThis.window.requestAnimationFrame) {
  globalThis.window.requestAnimationFrame = (cb) => {
    setTimeout(cb, 0);
    return 0;
  };
  globalThis.window.cancelAnimationFrame = () => {};
}
if (!globalThis.window.matchMedia) {
  globalThis.window.matchMedia = () => ({
    matches: false,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  });
}

// ── Tauri IPC interceptor ─────────────────────────────────────────────────────

const ipcHandlers = new Map();
function setIpcHandler(cmd, fn) {
  ipcHandlers.set(cmd, fn);
}
function clearIpcHandlers() {
  ipcHandlers.clear();
}

const tauriMock = {
  invoke(cmd, args) {
    const h = ipcHandlers.get(cmd);
    if (h) return h(args);
    return new Promise(() => {}); // pending — prevents unmocked-IPC errors
  },
  transformCallback(_cb) {
    return Math.random();
  },
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
if (globalThis.window && globalThis.window !== globalThis) {
  globalThis.window.__TAURI_INTERNALS__ = tauriMock;
}

// ── Minimal stub props for SettingsView ───────────────────────────────────────

const STUB_PROPS = {
  isUpdatingDesktopNotifications: false,
  notificationErrorMessage: null,
  notificationPermission: "denied",
  notificationSettings: {
    desktopNotificationsEnabled: false,
    homeBadgeEnabled: false,
    notifyWhileViewing: false,
    slotAlerts: {},
  },
  onSetDesktopNotificationsEnabled: async () => false,
  onSetHomeBadgeEnabled: () => {},
  onSetSlotAlertsEnabled: () => {},
  onSetNotifyWhileViewing: () => {},
  onSetAllSlotAlertsEnabled: () => {},
  onSetSoundForSlot: () => {},
};

// ── Harness ───────────────────────────────────────────────────────────────────

function makeQueryClient(pubkeyHex) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  qc.setQueryData(["identity"], { pubkey: pubkeyHex });
  return qc;
}

function mountSettingsView({
  section = "relay-admin",
  onSectionChange = () => {},
} = {}) {
  const qc = makeQueryClient("ab".repeat(32));
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  const doRender = async () => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: qc },
          React.createElement(
            SidebarProvider,
            {},
            React.createElement(SettingsView, {
              ...STUB_PROPS,
              section,
              onClose: () => {},
              onSectionChange,
            }),
          ),
        ),
      );
    });
  };

  const unmount = async () => {
    await act(async () => {
      root.unmount();
    });
    document.body.removeChild(container);
  };

  return { container, doRender, unmount };
}

async function settle(ms = 50) {
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

afterEach(() => {
  clearIpcHandlers();
});

// ── Core IPC stubs shared across all nav tests ────────────────────────────────
// These return the "no origin, no discovery" state so the nav resolution hook
// (if it were still active) would resolve to {originSource:"none"}.

function stubNoOrigin() {
  setIpcHandler("get_admin_origin", () => Promise.resolve(null));
  setIpcHandler("admin_discover_origin", () => Promise.resolve(null));
  setIpcHandler("get_relay_origin", () => Promise.resolve(null));
  setIpcHandler("get_relay_members_info", () => Promise.resolve(null));
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test("relay-admin-nav-visible-no-origin: nav renders when no origin saved and discovery returns null", async () => {
  // With the old predicate: hook resolves to {originSource:"none"} → hidden → RED.
  // With the current fix: nav is always present regardless of discovery state.
  stubNoOrigin();
  const { container, doRender, unmount } = mountSettingsView();
  try {
    await doRender();
    await settle();
    assert.ok(
      container.querySelector("[data-testid='settings-nav-relay-admin']"),
      "Relay admin nav must render when no origin and discovery returns null",
    );
  } finally {
    await unmount();
  }
});

test("relay-admin-nav-visible-discovery-error: nav renders when discovery IPCs throw", async () => {
  // Both origin lookup and discovery fail. Old code: "none" → hidden → RED.
  setIpcHandler("get_admin_origin", () =>
    Promise.reject(new Error("storage error")),
  );
  setIpcHandler("admin_discover_origin", () =>
    Promise.reject(new Error("network error")),
  );
  setIpcHandler("get_relay_origin", () => Promise.resolve(null));
  setIpcHandler("get_relay_members_info", () => Promise.resolve(null));
  const { container, doRender, unmount } = mountSettingsView();
  try {
    await doRender();
    await settle();
    assert.ok(
      container.querySelector("[data-testid='settings-nav-relay-admin']"),
      "Relay admin nav must render even when discovery errors",
    );
  } finally {
    await unmount();
  }
});

test("relay-admin-nav-visible-pending: nav renders while discovery IPC never resolves", async () => {
  // Hook stays undefined (disabled or pending). Old code: `moderationNav === undefined`
  // → false → nav hidden → RED. New code: nav is unconditional.
  setIpcHandler("get_admin_origin", () => new Promise(() => {}));
  setIpcHandler("admin_discover_origin", () => new Promise(() => {}));
  setIpcHandler("get_relay_origin", () => Promise.resolve(null));
  setIpcHandler("get_relay_members_info", () => Promise.resolve(null));
  const { container, doRender, unmount } = mountSettingsView();
  try {
    await doRender();
    await settle();
    assert.ok(
      container.querySelector("[data-testid='settings-nav-relay-admin']"),
      "Relay admin nav must render while discovery is pending",
    );
  } finally {
    await unmount();
  }
});

test("relay-admin-section-not-redirected: section=relay-admin is not normalized away after discovery", async () => {
  // Old code deferred section normalization until moderationNav resolved, then
  // redirected to appearance when origin was none.
  stubNoOrigin();
  const redirectedTo = [];
  const { container, doRender, unmount } = mountSettingsView({
    section: "relay-admin",
    onSectionChange: (s) => {
      if (s !== "relay-admin") redirectedTo.push(s);
    },
  });
  try {
    await doRender();
    await settle(80);
    assert.deepEqual(
      redirectedTo,
      [],
      `section=relay-admin must not be redirected; got redirects to: ${JSON.stringify(redirectedTo)}`,
    );
    assert.ok(
      container.querySelector("[data-testid='settings-nav-relay-admin']"),
      "Relay admin nav must still be present after settling",
    );
  } finally {
    await unmount();
  }
});

test("no-probe-before-save: admin_probe not called while rendering nav with no saved origin", async () => {
  // SettingsView no longer calls the nav resolution hook, so no probe hook
  // runs during nav rendering. Any probe before explicit Save is a trust boundary
  // violation.
  stubNoOrigin();
  let probeCalled = false;
  setIpcHandler("admin_probe", () => {
    probeCalled = true;
    return Promise.resolve({ state: "disabled" });
  });

  const { doRender, unmount } = mountSettingsView();
  try {
    await doRender();
    await settle(80);
    assert.equal(
      probeCalled,
      false,
      "admin_probe must not be called before an explicit Save",
    );
  } finally {
    await unmount();
  }
});

test("settings-nav-groups-contains-relay-admin: relay-admin is wired into Communities nav group", () => {
  const communities = settingsNavGroups.find((g) => g.label === "Communities");
  assert.ok(communities, "Communities group must exist");
  assert.ok(
    communities.sections.includes("relay-admin"),
    `relay-admin must be in Communities; got: ${JSON.stringify(communities.sections)}`,
  );
});
