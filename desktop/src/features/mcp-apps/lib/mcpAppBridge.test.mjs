import assert from "node:assert/strict";
import test from "node:test";

import {
  defaultMcpAppHostContext,
  mcpAppSandboxOrigin,
} from "./mcpAppBridge.ts";

test("derives an exact origin for the custom sandbox protocol", () => {
  assert.equal(
    mcpAppSandboxOrigin("buzz-mcp-app://localhost/7f6d"),
    "buzz-mcp-app://localhost",
  );
  assert.equal(
    mcpAppSandboxOrigin("http://buzz-mcp-app.localhost/7f6d"),
    "http://buzz-mcp-app.localhost",
  );
  assert.throws(
    () => mcpAppSandboxOrigin("https://example.com/7f6d"),
    /trusted protocol/,
  );
  assert.throws(
    () => mcpAppSandboxOrigin("buzz-mcp-app://localhost:1337/7f6d"),
    /trusted protocol/,
  );
  assert.throws(
    () => mcpAppSandboxOrigin("http://buzz-mcp-app.localhost:1337/7f6d"),
    /trusted protocol/,
  );
});

test("default host context identifies Buzz as a desktop host", () => {
  const originalDocument = globalThis.document;
  const originalNavigator = globalThis.navigator;
  const originalWindow = globalThis.window;
  globalThis.document = {
    documentElement: { classList: { contains: () => true } },
  };
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { language: "en-US", maxTouchPoints: 0 },
  });
  globalThis.window = {
    matchMedia: () => ({ matches: true }),
  };

  const context = defaultMcpAppHostContext();
  assert.equal(context.platform, "desktop");
  assert.equal(context.theme, "dark");
  assert.equal(context.locale, "en-US");
  assert.deepEqual(context.availableDisplayModes, ["inline"]);

  globalThis.document = originalDocument;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: originalNavigator,
  });
  globalThis.window = originalWindow;
});
