import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";
import { JSDOM } from "jsdom";

// Creation is unrelated to joining; avoid its template queries/native calls.
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "@/shared/theme/ThemeProvider")
      return { shortCircuit: true, url: "join-test:theme" };
    if (specifier === "@/features/sidebar/lib/useCreateChannelForm")
      return { shortCircuit: true, url: "join-test:form" };
    return next(specifier, context);
  },
  load(url, context, next) {
    if (url === "join-test:theme")
      return {
        shortCircuit: true,
        format: "module",
        source: "export const useTheme = () => ({isDark: false})",
      };
    if (url === "join-test:form")
      return {
        shortCircuit: true,
        format: "module",
        source: "export const useCreateChannelForm = () => ({})",
      };
    return next(url, context);
  },
});
const dom = new JSDOM(
  "<!doctype html><html><body><div id='root'></div></body></html>",
  { url: "https://buzz.test", pretendToBeVisual: true },
);
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Element: dom.window.Element,
  Node: dom.window.Node,
  NodeFilter: dom.window.NodeFilter,
  HTMLInputElement: dom.window.HTMLInputElement,
  MutationObserver: dom.window.MutationObserver,
  CustomEvent: dom.window.CustomEvent,
  Event: dom.window.Event,
  getComputedStyle: dom.window.getComputedStyle,
  requestAnimationFrame: dom.window.requestAnimationFrame.bind(dom.window),
  cancelAnimationFrame: dom.window.cancelAnimationFrame.bind(dom.window),
  ResizeObserver: class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
  IS_REACT_ACT_ENVIRONMENT: true,
});
document.fonts = { ready: Promise.resolve() };
dom.window.HTMLElement.prototype.scrollIntoView = () => {};
const { default: React, act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { ChannelBrowserDialog } = await import("./ChannelBrowserDialog.tsx");

test("channel browser exposes native string and Error join failures and allows retry", async () => {
  const root = createRoot(document.getElementById("root"));
  let attempts = 0;
  const selected = [];
  const opened = [];
  const props = {
    open: true,
    channels: [
      {
        id: "channel-1",
        name: "engineering",
        channelType: "stream",
        visibility: "open",
        description: "",
        isMember: false,
        memberCount: 0,
        memberPubkeys: [],
        participants: [],
        participantPubkeys: [],
        archivedAt: null,
        lastMessageAt: null,
      },
    ],
    onOpenChange: (value) => opened.push(value),
    onSelectChannel: (value) => selected.push(value),
    onJoinChannel: async () => {
      attempts++;
      if (attempts === 1)
        throw "This operation is disabled in the remote messaging preview";
      if (attempts === 2) throw new Error("Relay denied membership");
    },
  };
  const join = () =>
    document.querySelector(
      '[data-testid="browse-channel-engineering"] button:last-child',
    );
  try {
    await act(async () =>
      root.render(React.createElement(ChannelBrowserDialog, props)),
    );
    await act(async () => join().click());
    assert.match(
      document.querySelector('[role="alert"]').textContent,
      /disabled in the remote messaging preview/,
    );
    assert.deepEqual(opened, []);
    assert.deepEqual(selected, []);
    assert.equal(join().disabled, false);
    await act(async () => join().click());
    assert.match(
      document.querySelector('[role="alert"]').textContent,
      /Relay denied membership/,
    );
    await act(async () => join().click());
    assert.equal(document.querySelector('[role="alert"]'), null);
    assert.deepEqual(opened, [false]);
    assert.deepEqual(selected, ["channel-1"]);
  } finally {
    await act(async () => root.unmount());
    dom.window.close();
  }
});
