import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

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
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

function makeChannel(overrides) {
  return {
    id: "channel-1",
    name: "general",
    description: null,
    memberCount: 1,
    isMember: true,
    archivedAt: null,
    channelType: "stream",
    visibility: "open",
    participantPubkeys: [],
    ...overrides,
  };
}

test("getChannelRowIconKind returns 'lock' for private channels regardless of type", async () => {
  const { getChannelRowIconKind } = await import("./ChannelRowIcon.tsx");
  assert.equal(
    getChannelRowIconKind(makeChannel({ visibility: "private" })),
    "lock",
  );
  // A private forum still locks — the sidebar behaves the same way and the
  // two surfaces must not diverge (issue #6120).
  assert.equal(
    getChannelRowIconKind(
      makeChannel({ visibility: "private", channelType: "forum" }),
    ),
    "lock",
  );
});

test("getChannelRowIconKind returns 'forum' for public forum channels", async () => {
  const { getChannelRowIconKind } = await import("./ChannelRowIcon.tsx");
  assert.equal(
    getChannelRowIconKind(
      makeChannel({ channelType: "forum", visibility: "open" }),
    ),
    "forum",
  );
});

test("getChannelRowIconKind returns 'hash' for public streams", async () => {
  const { getChannelRowIconKind } = await import("./ChannelRowIcon.tsx");
  assert.equal(
    getChannelRowIconKind(
      makeChannel({ channelType: "stream", visibility: "open" }),
    ),
    "hash",
  );
});

test("ChannelRowIcon renders a Lock svg for private channels", async () => {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { ChannelRowIcon } = await import("./ChannelRowIcon.tsx");

  const { container } = render(
    createElement(ChannelRowIcon, {
      channel: makeChannel({ visibility: "private" }),
    }),
  );

  // Lucide icons render an inline svg with the relevant lucide-* class.
  assert.match(container.innerHTML, /lucide-lock/);
  assert.doesNotMatch(container.innerHTML, /lucide-file-text|lucide-hash/);
});

test("ChannelRowIcon renders a FileText svg for public forum channels", async () => {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { ChannelRowIcon } = await import("./ChannelRowIcon.tsx");

  const { container } = render(
    createElement(ChannelRowIcon, {
      channel: makeChannel({
        channelType: "forum",
        visibility: "open",
      }),
    }),
  );

  assert.match(container.innerHTML, /lucide-file-text/);
  assert.doesNotMatch(container.innerHTML, /lucide-lock|lucide-hash/);
});

test("ChannelRowIcon renders a Hash svg for public stream channels", async () => {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { ChannelRowIcon } = await import("./ChannelRowIcon.tsx");

  const { container } = render(
    createElement(ChannelRowIcon, {
      channel: makeChannel({
        channelType: "stream",
        visibility: "open",
      }),
    }),
  );

  assert.match(container.innerHTML, /lucide-hash/);
  assert.doesNotMatch(container.innerHTML, /lucide-lock|lucide-file-text/);
});