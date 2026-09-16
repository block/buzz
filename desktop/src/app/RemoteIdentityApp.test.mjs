import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";

// Keep the real remote entry component and onboarding controls; isolate native
// IPC and the workspace router (which otherwise starts the full application).
const stubs = {
  "@tauri-apps/api/core":
    "export const invoke = (...args) => globalThis.remoteUI.invoke(...args)",
  "@tauri-apps/api/event":
    "export const listen = async (name, callback) => { globalThis.remoteUI.listeners[name] = callback; return () => {}; }",
  "@/app/router": "export const router = {}",
  "@tanstack/react-router":
    'import React from "react"; export const RouterProvider = () => React.createElement("main", {id: "workspace"}, "Workspace content")',
  "@/features/agents/useKnownAgentPubkeys":
    "export const KnownAgentPubkeysProvider = ({children}) => children",
  "@/features/communities/useCommunities":
    "export const useCommunities = () => ({communities: []}); export const FixedCommunityProvider = ({children}) => children",
  "@/features/messages/lib/useDrafts": "export const initDraftStore = () => {}",
  "@/shared/api/nativeIdentitySession":
    "export const nativeIdentity = () => globalThis.remoteUI.identity; export const revokeNativeIdentity = () => {}",
  "@/features/onboarding/ui/LandingBees":
    "export const LandingBees = () => null",
  "@/shared/ui/StartupWindowDragRegion":
    "export const StartupWindowDragRegion = () => null",
};
const reactUrl = import.meta.resolve("react");
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "react") {
      return { shortCircuit: true, url: reactUrl };
    }
    return specifier in stubs
      ? { shortCircuit: true, url: `remote-ui:${specifier}` }
      : next(specifier, context);
  },
  load(url, context, next) {
    return url.startsWith("remote-ui:")
      ? { shortCircuit: true, format: "module", source: stubs[url.slice(10)] }
      : next(url, context);
  },
});
const { RemoteIdentityApp } = await import("./RemoteIdentityApp.tsx");

test("remote welcome uses Buzz branding; activation has no layout banner or sign-out UI", async () => {
  const dom = new JSDOM("<div id='root'></div>", { url: "https://buzz.test" });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    sessionStorage: dom.window.sessionStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const calls = [];
  globalThis.remoteUI = {
    identity: { authState: "signedOut", generation: 1 },
    listeners: {},
    invoke: async (command, args) => {
      calls.push([command, args]);
      return { ...remoteUI.identity, workspaceActive: true };
    },
  };
  let root = createRoot(document.getElementById("root"));
  try {
    await act(async () => root.render(React.createElement(RemoteIdentityApp)));
    assert.equal(
      document.querySelector('h1 img[alt="Buzz"]').getAttribute("src"),
      "/landing/buzz-wordmark.png",
    );
    assert.ok(document.querySelector(".buzz-onboarding-welcome"));
    assert.match(document.body.textContent, /Sign in with Block/);
    assert.doesNotMatch(document.body.textContent, /Sign out/);
    await act(async () => root.unmount());
    remoteUI.identity = {
      authState: "authenticated",
      generation: 2,
      publicIdentity: "a".repeat(64),
    };
    root = createRoot(document.getElementById("root"));
    await act(async () => root.render(React.createElement(RemoteIdentityApp)));
    assert.equal(
      document.querySelector("input").value,
      "wss://buzz.test.blockstaging.build",
    );
    await act(async () =>
      document
        .querySelector("form")
        .dispatchEvent(
          new dom.window.Event("submit", { bubbles: true, cancelable: true }),
        ),
    );
    assert.deepEqual(
      calls.find(([cmd]) => cmd === "activate_remote_workspace")[1],
      {
        relayUrl: "wss://buzz.test.blockstaging.build",
        expectedGeneration: 2,
      },
    );
    assert.ok(document.querySelector("#workspace"));
    assert.equal(document.querySelector("#workspace").parentElement.id, "root");
    assert.doesNotMatch(
      document.body.textContent,
      /Sign out|Staging remote identity/,
    );
    await act(async () => remoteUI.listeners["archive-sync-degraded"]());
    assert.match(
      document.querySelector('[role="alert"]').textContent,
      /reconnecting does not recover ephemeral gaps/,
    );
    assert.ok(
      document
        .querySelector('[role="alert"]')
        .parentElement.classList.contains("fixed"),
    );
    assert.equal(
      sessionStorage.getItem("buzz-remote-archive-degraded"),
      "true",
    );
  } finally {
    await act(async () => root.unmount());
    dom.window.close();
    delete globalThis.remoteUI;
  }
});
