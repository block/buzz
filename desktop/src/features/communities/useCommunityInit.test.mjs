import assert from "node:assert/strict";
import { after, afterEach, before, mock, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

let renderHook;
let waitFor;
let cleanup;
let act;
let useCommunityInit;
let relayClient;
const calls = [];
const pendingTrust = [];
let holdTrust = false;
const a = { id: "a", relayUrl: "wss://a.example", name: "A" };
const b = { id: "b", relayUrl: "wss://b.example", name: "B" };

before(async () => {
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "set_agent_avatar_communities" && holdTrust) {
        await new Promise((resolve, reject) => {
          pendingTrust.push({ resolve, reject });
        });
      }
      if (command === "get_identity") return { pubkey: "a".repeat(64) };
      if (command === "get_relay_url") return b.relayUrl;
      if (command === "enterprise_login_gate") return { status: "notRequired" };
      return undefined;
    },
    transformCallback: () => 1,
  };
  globalThis.__TAURI_INTERNALS__ = dom.window.__TAURI_INTERNALS__;
  ({ renderHook, waitFor, cleanup, act } = await import(
    "@testing-library/react"
  ));
  ({ useCommunityInit } = await import("./useCommunityInit.ts"));
  ({ relayClient } = await import("@/shared/api/relayClient"));
});

afterEach(async () => {
  cleanup();
  holdTrust = false;
  await act(async () => {
    for (const deferred of pendingTrust) deferred.resolve();
  });
  pendingTrust.length = 0;
  calls.length = 0;
  localStorage.clear();
});

after(() => dom.window.close());

function installTauriInvoke(handler) {
  const previousWindow = globalThis.window.__TAURI_INTERNALS__;
  const previousGlobal = globalThis.__TAURI_INTERNALS__;
  globalThis.window.__TAURI_INTERNALS__ = { invoke: handler };
  globalThis.__TAURI_INTERNALS__ = globalThis.window.__TAURI_INTERNALS__;
  return () => {
    if (previousWindow === undefined)
      delete globalThis.window.__TAURI_INTERNALS__;
    else globalThis.window.__TAURI_INTERNALS__ = previousWindow;
    if (previousGlobal === undefined) delete globalThis.__TAURI_INTERNALS__;
    else globalThis.__TAURI_INTERNALS__ = previousGlobal;
  };
}

function testCommunity(overrides = {}) {
  return {
    id: "community-1",
    name: "Enterprise",
    relayUrl: "wss://enterprise.example",
    token: "token-1",
    reposDir: "/tmp/buzz-repos",
    addedAt: "2026-09-17T00:00:00.000Z",
    ...overrides,
  };
}

function neverSettles() {
  return new Promise(() => {});
}

function mount(communities) {
  return renderHook((list) => useCommunityInit(b, "b", false, false, list), {
    initialProps: communities,
  });
}

test("useCommunityInit gates enterprise login before applying the community", async () => {
  const { cleanup, renderHook, waitFor } = await import(
    "@testing-library/react"
  );
  const { useCommunityInit } = await import("./useCommunityInit.ts");
  const calls = [];
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "get_identity") {
      return { pubkey: "pubkey-1", display_name: "Tester" };
    }
    if (command === "set_agent_avatar_communities") return null;
    if (command === "enterprise_login_gate") {
      return { status: "notRequired" };
    }
    if (command === "apply_workspace") {
      return null;
    }
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    const community = testCommunity();
    const hook = renderHook(() =>
      useCommunityInit(community, "community-key", false, true),
    );

    await waitFor(() => assert.equal(hook.result.current.isReady, true));

    assert.deepEqual(
      calls.map(([command]) => command),
      [
        "get_identity",
        "set_agent_avatar_communities",
        "enterprise_login_gate",
        "apply_workspace",
      ],
    );
    assert.deepEqual(calls[2], [
      "enterprise_login_gate",
      { relayUrl: community.relayUrl },
    ]);
    assert.deepEqual(calls[3], [
      "apply_workspace",
      {
        relayUrl: community.relayUrl,
        nsec: null,
        token: community.token,
        reposDir: community.reposDir,
        agentManagedProfiles: false,
      },
    ]);
    hook.unmount();
  } finally {
    restore();
    cleanup();
    mock.reset();
  }
});

test("useCommunityInit blocks community apply when enterprise login gate fails", async () => {
  const { cleanup, renderHook, waitFor } = await import(
    "@testing-library/react"
  );
  const { useCommunityInit } = await import("./useCommunityInit.ts");
  const consoleError = mock.method(console, "error", () => {});
  const calls = [];
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "get_identity") {
      return { pubkey: "pubkey-1", display_name: "Tester" };
    }
    if (command === "set_agent_avatar_communities") return null;
    if (command === "enterprise_login_gate") {
      throw new Error("enterprise login unavailable");
    }
    if (command === "apply_workspace") {
      throw new Error("apply_workspace must not run");
    }
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    const hook = renderHook(() =>
      useCommunityInit(testCommunity(), "community-key", false, true),
    );

    await waitFor(() =>
      assert.equal(hook.result.current.error, "enterprise login unavailable"),
    );

    assert.deepEqual(
      calls.map(([command]) => command),
      ["get_identity", "set_agent_avatar_communities", "enterprise_login_gate"],
    );
    assert.equal(consoleError.mock.calls.length, 1);
    hook.unmount();
  } finally {
    restore();
    cleanup();
    mock.reset();
  }
});

test("useCommunityInit cancels an owned pending Builderlab login on superseded init", async () => {
  const { cleanup, renderHook, waitFor, act } = await import(
    "@testing-library/react"
  );
  const { useCommunityInit } = await import("./useCommunityInit.ts");
  const calls = [];
  const communityA = testCommunity({
    id: "community-a",
    relayUrl: "wss://enterprise-a.example",
  });
  const communityB = testCommunity({
    id: "community-b",
    relayUrl: "wss://ordinary-b.example",
  });
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "get_identity") {
      return { pubkey: "pubkey-1", display_name: "Tester" };
    }
    if (command === "set_agent_avatar_communities") return null;
    if (command === "enterprise_login_gate") {
      return args.relayUrl === communityA.relayUrl
        ? { status: "required" }
        : { status: "notRequired" };
    }
    if (command === "get_builderlab_auth") {
      return null;
    }
    if (command === "start_builderlab_login") {
      return neverSettles();
    }
    if (command === "cancel_builderlab_login") {
      return null;
    }
    if (command === "apply_workspace") {
      return null;
    }
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    const hook = renderHook(
      ({ community, key }) => useCommunityInit(community, key, false, true),
      { initialProps: { community: communityA, key: "community-a-key" } },
    );

    await waitFor(() => assert.ok("enterpriseLogin" in hook.result.current));
    await act(async () => {
      hook.result.current.enterpriseLogin.onContinue();
    });
    await waitFor(() => {
      assert.equal(
        calls.some(([command]) => command === "start_builderlab_login"),
        true,
      );
    });
    const startCall = calls.find(
      ([command]) => command === "start_builderlab_login",
    );
    assert.equal(typeof startCall?.[1]?.attemptId, "string");

    hook.rerender({ community: communityB, key: "community-b-key" });

    await waitFor(() => assert.equal(hook.result.current.isReady, true));
    const cancelIndex = calls.findIndex(
      ([command]) => command === "cancel_builderlab_login",
    );
    const applyIndex = calls.findIndex(
      ([command, args]) =>
        command === "apply_workspace" && args.relayUrl === communityB.relayUrl,
    );

    assert.notEqual(cancelIndex, -1);
    assert.notEqual(applyIndex, -1);
    assert.ok(cancelIndex < applyIndex);
    assert.deepEqual(calls[cancelIndex], [
      "cancel_builderlab_login",
      { attemptId: startCall[1].attemptId },
    ]);
    assert.equal(
      calls.some(
        ([command, args]) =>
          command === "apply_workspace" &&
          args.relayUrl === communityA.relayUrl,
      ),
      false,
    );
    hook.unmount();
  } finally {
    restore();
    cleanup();
    mock.reset();
  }
});

test("useCommunityInit waits for explicit enterprise browser consent", async () => {
  const { cleanup, renderHook, waitFor, act } = await import(
    "@testing-library/react"
  );
  const { useCommunityInit } = await import("./useCommunityInit.ts");
  const calls = [];
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "get_identity") {
      return { pubkey: "pubkey-1", display_name: "Tester" };
    }
    if (command === "set_agent_avatar_communities") return null;
    if (command === "enterprise_login_gate") return { status: "required" };
    if (command === "get_builderlab_auth") return null;
    if (command === "start_builderlab_login") {
      return { expiresAt: "2026-09-18T21:00:00Z" };
    }
    if (command === "apply_workspace") return null;
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    const community = testCommunity({ name: "Block Buzz" });
    const hook = renderHook(() =>
      useCommunityInit(community, "community-key", false, true),
    );

    await waitFor(() => assert.ok("enterpriseLogin" in hook.result.current));
    assert.equal(
      hook.result.current.enterpriseLogin.communityName,
      "Block Buzz",
    );
    assert.deepEqual(
      calls.map(([command]) => command),
      [
        "get_identity",
        "set_agent_avatar_communities",
        "enterprise_login_gate",
        "get_builderlab_auth",
      ],
    );

    await act(async () => {
      hook.result.current.enterpriseLogin.onContinue();
    });
    await waitFor(() => assert.equal(hook.result.current.isReady, true));

    assert.deepEqual(
      calls.map(([command]) => command),
      [
        "get_identity",
        "set_agent_avatar_communities",
        "enterprise_login_gate",
        "get_builderlab_auth",
        "start_builderlab_login",
        "apply_workspace",
      ],
    );
    hook.unmount();
  } finally {
    restore();
    cleanup();
    mock.reset();
  }
});

test("useCommunityInit exposes authoritative enterprise profile when both identity profile fields are present", async () => {
  const { cleanup, renderHook, waitFor, act } = await import(
    "@testing-library/react"
  );
  const { useCommunityInit } = await import("./useCommunityInit.ts");
  const calls = [];
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "get_identity") {
      return { pubkey: "pubkey-1", display_name: "Tester" };
    }
    if (command === "set_agent_avatar_communities") return null;
    if (command === "enterprise_login_gate") return { status: "required" };
    if (command === "get_builderlab_auth") return null;
    if (command === "start_builderlab_login") {
      return {
        expiresAt: "2026-09-18T21:00:00Z",
        username: " seiler ",
        name: " Brad Seiler ",
      };
    }
    if (command === "apply_workspace") return null;
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    const hook = renderHook(() =>
      useCommunityInit(testCommunity(), "community-key", false, true),
    );
    await waitFor(() => assert.ok("enterpriseLogin" in hook.result.current));
    await act(async () => {
      hook.result.current.enterpriseLogin.onContinue();
    });
    await waitFor(() => assert.equal(hook.result.current.isReady, true));
    assert.deepEqual(hook.result.current.enterpriseProfile, {
      username: "seiler",
      displayName: "Brad Seiler",
    });
    hook.unmount();
  } finally {
    restore();
    cleanup();
    mock.reset();
  }
});

test("inactive relay removal/edit/add refreshes trust without reapplying or disconnecting active community", async () => {
  let disconnects = 0;
  const original = relayClient.disconnect;
  relayClient.disconnect = () => {
    disconnects += 1;
  };
  try {
    const { result, rerender } = mount([a, b]);
    await waitFor(() => assert.equal(result.current.isReady, true));
    assert.equal(calls.filter(([cmd]) => cmd === "apply_workspace").length, 1);
    for (const list of [
      [b],
      [{ ...a, relayUrl: "wss://new-a.example" }, b],
      [a, b],
    ]) {
      const before = calls.filter(
        ([cmd]) => cmd === "set_agent_avatar_communities",
      ).length;
      rerender(list);
      await waitFor(() =>
        assert.equal(
          calls.filter(([cmd]) => cmd === "set_agent_avatar_communities")
            .length,
          before + 1,
        ),
      );
      assert.equal(result.current.isReady, true);
      assert.equal(
        calls.filter(([cmd]) => cmd === "apply_workspace").length,
        1,
      );
      assert.equal(disconnects, 0);
    }
    const count = calls.length;
    rerender([{ ...b, name: "New label" }, a]);
    assert.equal(calls.length, count);
    const updates = calls.filter(
      ([cmd]) => cmd === "set_agent_avatar_communities",
    );
    assert.deepEqual(updates[1][1].relayUrls, ["https://b.example"]);
  } finally {
    relayClient.disconnect = original;
  }
});

test("initial workspace restore waits for avatar trust IPC", async () => {
  holdTrust = true;
  const { result } = mount([a, b]);
  await waitFor(() => assert.equal(pendingTrust.length, 1));
  // Let identity resolution and any unguarded apply settle while trust is held.
  await act(async () => {});
  assert.equal(
    calls.some(([cmd]) => cmd === "apply_workspace"),
    false,
  );
  await act(async () => pendingTrust[0].resolve());
  await waitFor(() => assert.equal(result.current.isReady, true));
  assert.equal(calls.filter(([cmd]) => cmd === "apply_workspace").length, 1);
});

test("source removal during pending trust serializes IPC and blocks restore until the latest update", async (t) => {
  holdTrust = true;
  const disconnect = t.mock.method(relayClient, "disconnect", () => {});
  const { result, rerender } = mount([a, b]);
  await waitFor(() => assert.equal(pendingTrust.length, 1));
  await act(async () => {});

  rerender([b]);
  await act(async () => {});
  assert.equal(
    pendingTrust.length,
    1,
    "P2 must not dispatch before P1 settles",
  );
  assert.equal(result.current.isReady, false);

  await act(async () => pendingTrust[0].resolve());
  await waitFor(() => assert.equal(pendingTrust.length, 2));
  assert.deepEqual(
    calls
      .filter(([cmd]) => cmd === "set_agent_avatar_communities")
      .map(([, args]) => args.relayUrls),
    [["https://a.example", "https://b.example"], ["https://b.example"]],
  );
  assert.equal(
    calls.filter(([cmd]) => cmd === "apply_workspace").length,
    0,
    "P1 alone must not release restoration while source removal is pending",
  );
  assert.equal(result.current.isReady, false);

  await act(async () => pendingTrust[1].resolve());
  await waitFor(() => assert.equal(result.current.isReady, true));
  assert.equal(calls.filter(([cmd]) => cmd === "apply_workspace").length, 1);
  assert.equal(disconnect.mock.callCount(), 0);
});

for (const failingUpdate of [0, 1]) {
  test(`trust update ${failingUpdate + 1} rejection does not release queued restoration`, async (t) => {
    holdTrust = true;
    t.mock.method(console, "error", () => {});
    const { result, rerender } = mount([a, b]);
    await waitFor(() => assert.equal(pendingTrust.length, 1));
    rerender([b]);
    await act(async () => {});
    if (failingUpdate === 1) {
      await act(async () => pendingTrust[0].resolve());
      await waitFor(() => assert.equal(pendingTrust.length, 2));
    }
    await act(async () =>
      pendingTrust[failingUpdate].reject(new Error("IPC failed")),
    );
    assert.equal(result.current.isReady, false);
    assert.ok(result.current.error);
    assert.equal(calls.filter(([cmd]) => cmd === "apply_workspace").length, 0);
    assert.equal(pendingTrust.length, failingUpdate + 1);
  });
}
