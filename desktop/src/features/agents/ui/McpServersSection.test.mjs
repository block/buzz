/**
 * #3493 provenance: the MCP servers section must attribute its entries to the
 * ACTUAL config file the reader read — which, under a custom CLAUDE_CONFIG_DIR,
 * is the isolated `<custom>/.claude.json`, not the default `~/.claude.json`.
 *
 * Before this fix `mcpConfigFilePath` was carried on the DTO but no component
 * consumed it, so the panel listed the correct servers with no file
 * attribution at all. These pin the rendered contract.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

import {
  McpServersSection,
  mcpConfigFileCaption,
} from "./McpServersSection.tsx";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

const CLAUDE_MCP = [{ name: "sentinel", kind: "stdio", enabled: true }];

test("mcpConfigFileCaption_customPath_returnsFileAttribution", () => {
  assert.equal(
    mcpConfigFileCaption("/tmp/iso/.claude.json"),
    "From config file (/tmp/iso/.claude.json)",
  );
});

test("mcpConfigFileCaption_nullPath_returnsNull", () => {
  assert.equal(mcpConfigFileCaption(null), null);
  assert.equal(mcpConfigFileCaption(undefined), null);
});

test("McpServersSection_customConfigDir_rendersIsolatedFilePath", async () => {
  const { render } = await import("@testing-library/react");
  const React = await import("react");

  const { container } = render(
    React.createElement(McpServersSection, {
      extensions: CLAUDE_MCP,
      mcpConfigFilePath: "/tmp/iso/.claude.json",
      runtimeId: "claude",
      variant: "compact",
    }),
  );

  assert.match(container.textContent, /sentinel/);
  assert.match(
    container.textContent,
    /From config file \(\/tmp\/iso\/\.claude\.json\)/,
  );
});

test("McpServersSection_noConfigPath_omitsFileAttribution", async () => {
  const { render } = await import("@testing-library/react");
  const React = await import("react");

  const { container } = render(
    React.createElement(McpServersSection, {
      extensions: CLAUDE_MCP,
      mcpConfigFilePath: null,
      runtimeId: "claude",
      variant: "compact",
    }),
  );

  assert.match(container.textContent, /sentinel/);
  assert.doesNotMatch(container.textContent, /From config file/);
});
