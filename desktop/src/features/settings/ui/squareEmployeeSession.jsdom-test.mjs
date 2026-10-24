import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { fireEvent } from "@testing-library/react";
import { relayClient } from "@/shared/api/relayClient";
import { SquareEmployeeSessionGate } from "./SquareEmployeeSessionGate.tsx";
import { SquareEmployeeSignInCard } from "./SquareEmployeeSignInCard.tsx";

const handlers = new Map();
const calls = [];
const callbacks = new Map();
const listeners = new Map();
let nextId = 0;
const ipc = {
  invoke(command, args) {
    calls.push([command, args]);
    if (command === "plugin:event|listen") {
      const id = ++nextId;
      listeners.set(id, {
        event: args.event,
        callback: callbacks.get(args.handler),
      });
      return Promise.resolve(id);
    }
    if (command === "plugin:event|unlisten") {
      listeners.delete(args.eventId);
      return Promise.resolve();
    }
    return (
      handlers.get(command)?.(args) ??
      Promise.reject(new Error(`Unexpected command: ${command}`))
    );
  },
  transformCallback(callback) {
    const id = ++nextId;
    callbacks.set(id, callback);
    return id;
  },
  unregisterCallback(id) {
    callbacks.delete(id);
  },
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener(_event, id) {
    listeners.delete(id);
  },
};
window.__TAURI_INTERNALS__ = ipc;
globalThis.__TAURI_INTERNALS__ = ipc;
const originalDisconnect = relayClient.disconnect;
const originalReconnect = relayClient.reconnect;
let root;
let container;
const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};
const settle = async () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
async function mount(element) {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(element));
  await settle();
}
async function click(label) {
  const button = [...container.querySelectorAll("button")].find(
    (element) => element.textContent === label,
  );
  assert.ok(button, `Missing button: ${label}`);
  await act(async () => fireEvent.click(button));
  await settle();
}
const gate = (scope) =>
  React.createElement(
    SquareEmployeeSessionGate,
    { active: true, scope },
    React.createElement("p", null, "Community chat"),
  );
beforeEach(() => {
  handlers.set("employee_session_status", async () => ({
    required: true,
    authenticated: false,
    expiresAt: null,
  }));
});
afterEach(async () => {
  if (root) await act(async () => root.unmount());
  container?.remove();
  root = undefined;
  handlers.clear();
  calls.length = 0;
  relayClient.disconnect = originalDisconnect;
  relayClient.reconnect = originalReconnect;
  assert.equal(listeners.size, 0, "native session listeners are released");
  callbacks.clear();
});

test("ordinary communities retain the existing onboarding path", async () => {
  handlers.set("employee_session_status", async () => ({
    required: false,
    authenticated: false,
    expiresAt: null,
  }));
  await mount(gate("ordinary"));
  assert.match(container.textContent, /Community chat/);
  assert.equal(
    calls.filter(([name]) => name === "employee_sign_in_available").length,
    0,
  );
});

test("employee sign-in is reachable before relay-dependent onboarding", async () => {
  handlers.set("employee_session_status", async () => ({
    required: true,
    authenticated: false,
    expiresAt: null,
  }));
  handlers.set("employee_sign_in_available", async () => true);
  await mount(gate("employee"));
  assert.doesNotMatch(container.textContent, /Community chat/);
  assert.match(container.textContent, /Sign in with Square/);
});

test("a late access check from a previous community cannot unlock the current one", async () => {
  const old = deferred();
  let count = 0;
  handlers.set("employee_session_status", () =>
    ++count === 1
      ? old.promise
      : Promise.resolve({
          required: true,
          authenticated: false,
          expiresAt: null,
        }),
  );
  handlers.set("employee_sign_in_available", async () => true);
  await mount(gate("old"));
  await act(async () => root.render(gate("current")));
  await settle();
  await act(async () =>
    old.resolve({ required: false, authenticated: false, expiresAt: null }),
  );
  await settle();
  assert.doesNotMatch(container.textContent, /Community chat/);
  assert.match(container.textContent, /Sign in with Square/);
});

test("an access check failure can be retried without opening chat", async () => {
  handlers.set("employee_session_status", async () => {
    throw new Error("offline");
  });
  await mount(gate("employee"));
  assert.match(container.textContent, /Square access could not be checked/);
  assert.doesNotMatch(container.textContent, /Community chat/);
  handlers.set("employee_session_status", async () => ({
    required: true,
    authenticated: true,
    expiresAt: Math.floor(Date.now() / 1000) + 240,
  }));
  await click("Retry access check");
  assert.match(container.textContent, /Community chat/);
});

test("cancel fences a late login result before relay connection", async () => {
  const login = deferred();
  let connections = 0;
  handlers.set("employee_sign_in_available", async () => true);
  handlers.set("start_employee_login", () => login.promise);
  handlers.set("cancel_builderlab_login", async () => undefined);
  relayClient.disconnect = () => {};
  relayClient.reconnect = async () => {
    connections += 1;
  };
  await mount(React.createElement(SquareEmployeeSignInCard));
  await click("Sign in with Square");
  await click("Cancel sign-in");
  await act(async () => login.resolve({ expires_at: "public expiry" }));
  await settle();
  assert.equal(connections, 0);
  assert.doesNotMatch(
    container.textContent,
    /Square session and your Buzz key are connected/,
  );
  assert.equal(container.textContent.includes("Cancel sign-in"), false);
});

test("community refusal keeps an explicit retry, and sign-out tears down native access", async () => {
  let connects = 0;
  let disconnects = 0;
  let connected = 0;
  handlers.set("employee_sign_in_available", async () => true);
  let authenticated = false;
  handlers.set("employee_session_status", async () => ({
    required: true,
    authenticated,
    expiresAt: authenticated ? Math.floor(Date.now() / 1000) + 240 : null,
  }));
  handlers.set("start_employee_login", async () => {
    authenticated = true;
    return { expires_at: "public expiry" };
  });
  handlers.set("refresh_employee_relay_access", async () => undefined);
  handlers.set("clear_builderlab_auth", async () => {
    authenticated = false;
  });
  relayClient.disconnect = () => {
    disconnects += 1;
  };
  relayClient.reconnect = async () => {
    if (++connects === 1) throw new Error("Community admission is pending");
  };
  await mount(
    React.createElement(SquareEmployeeSignInCard, {
      onConnected: () => {
        connected += 1;
      },
    }),
  );
  await click("Sign in with Square");
  assert.match(container.textContent, /Community admission is pending/);
  assert.equal(connected, 0);
  await click("Retry community connection");
  assert.equal(connected, 1);
  await click("Sign out of Square");
  assert.equal(
    disconnects,
    1,
    "only sign-out tears down subscription ownership",
  );
  assert.equal(connects, 2, "login and retry preserve live owners");
  assert.equal(
    calls.filter(([name]) => name === "clear_builderlab_auth").length,
    1,
  );
  assert.doesNotMatch(
    container.textContent,
    /Square session and your Buzz key are connected/,
  );
});

async function nativeSessionChanged() {
  await act(async () => {
    for (const [id, listener] of [...listeners])
      if (listener.event === "builderlab-session-changed")
        listener.callback({ event: listener.event, id, payload: null });
  });
  await settle();
}

test("settings hydrates existing native access and every view reacts to shared sign-out", async () => {
  let authenticated = true;
  handlers.set("employee_sign_in_available", async () => true);
  handlers.set("employee_session_status", async () => ({
    required: true,
    authenticated,
    expiresAt: authenticated ? Math.floor(Date.now() / 1000) + 240 : null,
  }));
  await mount(
    React.createElement(
      React.Fragment,
      null,
      gate("employee"),
      React.createElement(SquareEmployeeSignInCard),
    ),
  );
  assert.match(container.textContent, /Community chat/);
  assert.match(container.textContent, /Sign out of Square/);
  authenticated = false;
  await nativeSessionChanged();
  assert.doesNotMatch(container.textContent, /Community chat/);
  assert.doesNotMatch(container.textContent, /Sign out of Square/);
  assert.match(container.textContent, /Sign in with Square/);
});

test("native change closes content during recheck and ignores an older successful response", async () => {
  handlers.set("employee_sign_in_available", async () => true);
  handlers.set("employee_session_status", async () => ({
    required: true,
    authenticated: true,
    expiresAt: Math.floor(Date.now() / 1000) + 240,
  }));
  await mount(gate("employee"));
  assert.match(container.textContent, /Community chat/);
  const pending = deferred();
  handlers.set("employee_session_status", () => pending.promise);
  await nativeSessionChanged();
  assert.doesNotMatch(container.textContent, /Community chat/);
  handlers.set("employee_session_status", async () => ({
    required: true,
    authenticated: false,
    expiresAt: null,
  }));
  await nativeSessionChanged();
  await act(async () =>
    pending.resolve({
      required: true,
      authenticated: true,
      expiresAt: Math.floor(Date.now() / 1000) + 240,
    }),
  );
  await settle();
  assert.doesNotMatch(container.textContent, /Community chat/);
  assert.match(container.textContent, /Sign in with Square/);
});

test("local community and identity recovery stay reachable while access is refused", async () => {
  let switched = 0,
    recovered = 0,
    canceled = 0;
  handlers.set("employee_sign_in_available", async () => true);
  const recovery = React.createElement(
    "nav",
    { "aria-label": "Community recovery" },
    React.createElement(
      "button",
      { type: "button", onClick: () => switched++ },
      "Switch to saved community",
    ),
    React.createElement(
      "button",
      { type: "button", onClick: () => recovered++ },
      "Recover Buzz identity",
    ),
    React.createElement(
      "button",
      { type: "button", onClick: () => canceled++ },
      "Cancel joining community",
    ),
  );
  await mount(
    React.createElement(
      SquareEmployeeSessionGate,
      { active: true, scope: "employee", recovery },
      React.createElement("p", null, "Community chat"),
    ),
  );
  await click("Switch to saved community");
  await click("Recover Buzz identity");
  await click("Cancel joining community");
  assert.deepEqual([switched, recovered, canceled], [1, 1, 1]);
  assert.doesNotMatch(container.textContent, /Community chat/);
});

test("refused relay access retains an explicit retry without claiming the employee is connected", async () => {
  handlers.set("employee_sign_in_available", async () => true);
  handlers.set("employee_session_status", async () => ({
    required: true,
    authenticated: false,
    canRetry: true,
    expiresAt: null,
  }));
  handlers.set("refresh_employee_relay_access", async () => {});
  relayClient.reconnect = async () => {};
  await mount(React.createElement(SquareEmployeeSignInCard));
  assert.match(container.textContent, /Retry community connection/);
  assert.doesNotMatch(
    container.textContent,
    /Square session and your Buzz key are connected/,
  );
  await click("Retry community connection");
  assert.ok(
    calls.some(([command]) => command === "refresh_employee_relay_access"),
  );
  assert.ok(!calls.some(([command]) => command === "start_employee_login"));
});
