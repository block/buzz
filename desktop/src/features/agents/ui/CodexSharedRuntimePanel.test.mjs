import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React from "react";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
globalThis.window = dom.window;
globalThis.document = dom.window.document;
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});
globalThis.HTMLElement = dom.window.HTMLElement;
globalThis.Node = dom.window.Node;
globalThis.Event = dom.window.Event;
globalThis.CustomEvent = dom.window.CustomEvent;
globalThis.MutationObserver = dom.window.MutationObserver;
globalThis.getComputedStyle = dom.window.getComputedStyle;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.PointerEvent = dom.window.PointerEvent ?? dom.window.MouseEvent;
globalThis.ResizeObserver = class {
  disconnect() {}
  observe() {}
  unobserve() {}
};
dom.window.HTMLElement.prototype.hasPointerCapture ??= () => false;
dom.window.HTMLElement.prototype.releasePointerCapture ??= () => {};
dom.window.HTMLElement.prototype.setPointerCapture ??= () => {};
dom.window.HTMLElement.prototype.scrollIntoView ??= () => {};

const { cleanup, fireEvent, render, screen, waitFor } = await import(
  "@testing-library/react"
);
const { CodexSharedRuntimePanel } = await import(
  "./CodexSharedRuntimePanel.tsx"
);

const CONFLICT_STATUS = {
  enabled: true,
  state: "ready",
  url: "ws://127.0.0.1:51919",
  detail: null,
  desktop_process_ids: [100],
  private_app_server_process_ids: [101],
  desktop_detection_error: null,
};

const RESOLVED_STATUS = {
  ...CONFLICT_STATUS,
  desktop_process_ids: [200],
  private_app_server_process_ids: [],
};

function codexRuntime(availability) {
  return {
    id: "codex",
    label: "Codex",
    avatar_url: "",
    availability,
    command: availability === "available" ? "codex-acp" : null,
    binary_path:
      availability === "available" ? "C:\\Buzz\\codex-acp.cmd" : null,
    default_args: [],
    mcp_command: "buzz-dev-mcp",
    install_hint: "Install Codex ACP",
    install_instructions_url:
      "https://github.com/agentclientprotocol/codex-acp",
    can_auto_install: true,
    requires_external_cli: true,
    underlying_cli_path: "C:\\Codex\\codex.exe",
    node_required: availability !== "available",
    auth_status: { status: "logged_in" },
    login_hint: null,
    source: "builtin",
    definition_env: {},
  };
}

test("conflict takeover requires confirmation and refreshes status", async (t) => {
  let takeoverCalls = 0;
  window.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      if (command === "get_codex_shared_runtime_status") {
        return Promise.resolve(CONFLICT_STATUS);
      }
      if (command === "discover_acp_providers") {
        return Promise.resolve([codexRuntime("available")]);
      }
      if (command === "take_over_codex_desktop_shared") {
        takeoverCalls += 1;
        assert.deepEqual(args, { confirmed: true });
        return Promise.resolve(RESOLVED_STATUS);
      }
      return Promise.reject(new Error(`unexpected Tauri command: ${command}`));
    },
  };
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  t.after(() => {
    cleanup();
    client.clear();
    delete window.__TAURI_INTERNALS__;
  });

  render(
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(CodexSharedRuntimePanel, { enabled: true }),
    ),
  );

  await screen.findByText("Codex Desktop runtime conflict");
  const takeover = screen.getByRole("button", {
    name: "Take over Codex Desktop",
  });
  fireEvent.click(takeover);
  assert.match(
    screen.getByText(/Closing it may stop active turns/).textContent,
    /ws:\/\/127\.0\.0\.1:51919/,
  );

  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  assert.equal(takeoverCalls, 0);

  fireEvent.click(takeover);
  fireEvent.click(screen.getByRole("button", { name: "Close and reconnect" }));
  await waitFor(() => assert.equal(takeoverCalls, 1));
  await screen.findByText("Codex shared runtime connected");
  assert.equal(screen.queryByText("Codex Desktop runtime conflict"), null);
});

test("setup installs Codex ACP before enabling the shared runtime", async (t) => {
  const commands = [];
  let installed = false;
  window.__TAURI_INTERNALS__ = {
    invoke(command) {
      commands.push(command);
      if (command === "get_codex_shared_runtime_status") {
        return Promise.resolve({
          ...RESOLVED_STATUS,
          enabled: false,
          state: "disabled",
        });
      }
      if (command === "discover_acp_providers") {
        return Promise.resolve([
          codexRuntime(installed ? "available" : "adapter_missing"),
        ]);
      }
      if (command === "install_acp_runtime") {
        installed = true;
        return Promise.resolve({
          success: true,
          steps: [],
          restarted_count: 0,
          failed_restart_count: 0,
          log_path: null,
        });
      }
      if (command === "enable_codex_shared_runtime") {
        return Promise.resolve(RESOLVED_STATUS);
      }
      return Promise.reject(new Error(`unexpected Tauri command: ${command}`));
    },
  };
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  t.after(() => {
    cleanup();
    client.clear();
    delete window.__TAURI_INTERNALS__;
  });

  render(
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(CodexSharedRuntimePanel, { enabled: true }),
    ),
  );

  const setup = await screen.findByRole("button", { name: "Set up Codex" });
  await waitFor(() => assert.equal(setup.disabled, false));
  fireEvent.click(setup);
  await waitFor(() => {
    assert.deepEqual(
      commands.filter((command) =>
        ["install_acp_runtime", "enable_codex_shared_runtime"].includes(
          command,
        ),
      ),
      ["install_acp_runtime", "enable_codex_shared_runtime"],
    );
  });
  await screen.findByText("Codex shared runtime connected");

  const installIndex = commands.indexOf("install_acp_runtime");
  const enableIndex = commands.indexOf("enable_codex_shared_runtime");
  assert.ok(installIndex >= 0);
  assert.ok(enableIndex > installIndex);
});
