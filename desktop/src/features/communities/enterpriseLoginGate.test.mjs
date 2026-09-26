import assert from "node:assert/strict";
import test from "node:test";

const gate = await import("./enterpriseLoginGate.ts");

function installTauriInvoke(handler) {
  if (typeof globalThis.window === "undefined") {
    Object.defineProperty(globalThis, "window", {
      value: globalThis,
      configurable: true,
    });
  }
  const previous = globalThis.window.__TAURI_INTERNALS__;
  globalThis.window.__TAURI_INTERNALS__ = { invoke: handler };
  return () => {
    if (previous === undefined) delete globalThis.window.__TAURI_INTERNALS__;
    else globalThis.window.__TAURI_INTERNALS__ = previous;
  };
}

test("enterpriseLoginGate forwards the selected relay URL to native discovery", async () => {
  const calls = [];
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    return { status: "notRequired" };
  });

  try {
    assert.deepEqual(await gate.enterpriseLoginGate("wss://relay.example"), {
      status: "notRequired",
    });
    assert.deepEqual(calls, [
      ["enterprise_login_gate", { relayUrl: "wss://relay.example" }],
    ]);
  } finally {
    restore();
  }
});

test("ensureEnterpriseLoginForRelay skips enterprise auth when enterprise login is not required", async () => {
  const calls = [];
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "enterprise_login_gate") return { status: "notRequired" };
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    assert.equal(
      await gate.ensureEnterpriseLoginForRelay("wss://relay.example"),
      null,
    );
    assert.deepEqual(calls, [
      ["enterprise_login_gate", { relayUrl: "wss://relay.example" }],
    ]);
  } finally {
    restore();
  }
});

test("ensureEnterpriseLoginForRelay reuses an existing enterprise auth session", async () => {
  const auth = {
    email: "person@example.com",
    expiresAt: "2026-09-17T17:00:00Z",
  };
  const calls = [];
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "enterprise_login_gate") return { status: "required" };
    if (command === "get_enterprise_auth") return auth;
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    assert.deepEqual(
      await gate.ensureEnterpriseLoginForRelay("wss://relay.example"),
      auth,
    );
    assert.deepEqual(
      calls.map(([command]) => command),
      ["enterprise_login_gate", "get_enterprise_auth"],
    );
  } finally {
    restore();
  }
});

test("ensureEnterpriseLoginForRelay starts browser login when no valid session exists", async () => {
  const auth = {
    email: "person@example.com",
    expiresAt: "2026-09-17T17:00:00Z",
  };
  const calls = [];
  const started = [];
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "enterprise_login_gate") return { status: "required" };
    if (command === "get_enterprise_auth") throw new Error("expired");
    if (command === "start_enterprise_auth_login") return auth;
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    assert.deepEqual(
      await gate.ensureEnterpriseLoginForRelay("wss://relay.example", {
        loginAttemptId: "attempt-1",
        onBrowserLoginStarted: () => started.push("started"),
      }),
      auth,
    );
    assert.deepEqual(
      calls.map(([command]) => command),
      [
        "enterprise_login_gate",
        "get_enterprise_auth",
        "start_enterprise_auth_login",
      ],
    );
    assert.deepEqual(calls.at(-1), [
      "start_enterprise_auth_login",
      { attemptId: "attempt-1" },
    ]);
    assert.deepEqual(started, ["started"]);
  } finally {
    restore();
  }
});

test("ensureEnterpriseLoginForRelay waits for explicit consent before browser login", async () => {
  const calls = [];
  let continueLogin;
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "enterprise_login_gate") return { status: "required" };
    if (command === "get_enterprise_auth") return null;
    if (command === "start_enterprise_auth_login") {
      return { expiresAt: "2026-09-18T21:00:00Z" };
    }
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    const pending = gate.ensureEnterpriseLoginForRelay("wss://relay.example", {
      onEnterpriseLoginRequired: () =>
        new Promise((resolve) => {
          continueLogin = resolve;
        }),
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.deepEqual(
      calls.map(([command]) => command),
      ["enterprise_login_gate", "get_enterprise_auth"],
    );
    continueLogin(true);
    await pending;
    assert.deepEqual(
      calls.map(([command]) => command),
      [
        "enterprise_login_gate",
        "get_enterprise_auth",
        "start_enterprise_auth_login",
      ],
    );
  } finally {
    restore();
  }
});

test("ensureEnterpriseLoginForRelay does not open browser when consent is canceled", async () => {
  const calls = [];
  const restore = installTauriInvoke(async (command, args) => {
    calls.push([command, args]);
    if (command === "enterprise_login_gate") return { status: "required" };
    if (command === "get_enterprise_auth") return null;
    throw new Error(`unexpected command: ${command}`);
  });

  try {
    await assert.rejects(
      gate.ensureEnterpriseLoginForRelay("wss://relay.example", {
        onEnterpriseLoginRequired: () => false,
      }),
      /Enterprise sign-in canceled/,
    );
    assert.deepEqual(
      calls.map(([command]) => command),
      ["enterprise_login_gate", "get_enterprise_auth"],
    );
  } finally {
    restore();
  }
});
