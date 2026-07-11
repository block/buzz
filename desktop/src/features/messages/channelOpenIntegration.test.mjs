import assert from "node:assert/strict";
import test from "node:test";
function installDOMShim() {
  class MinimalEventTarget {
    constructor() {
      this._listeners = {};
    }
    addEventListener(type, fn) {
      this._listeners[type] ??= [];
      this._listeners[type].push(fn);
    }
    removeEventListener(type, fn) {
      if (this._listeners[type]) {
        this._listeners[type] = this._listeners[type].filter((f) => f !== fn);
      }
    }
    dispatchEvent(e) {
      for (const fn of this._listeners[e.type] ?? []) fn(e);
      return true;
    }
  }

  class MinimalNode extends MinimalEventTarget {
    constructor(tagName) {
      super();
      this.tagName = tagName;
      this.children = [];
      this.childNodes = [];
      this.style = {};
      this.nodeType = 1;
      this.parentNode = null;
    }
    get ownerDocument() {
      return globalThis.document;
    }
    get firstChild() {
      return this.children[0] ?? null;
    }
    get lastChild() {
      return this.children[this.children.length - 1] ?? null;
    }
    get nextSibling() {
      return null;
    }
    get nodeValue() {
      return null;
    }
    appendChild(child) {
      this.children.push(child);
      this.childNodes.push(child);
      child.parentNode = this;
      return child;
    }
    removeChild(child) {
      this.children = this.children.filter((c) => c !== child);
      this.childNodes = this.childNodes.filter((c) => c !== child);
      return child;
    }
    insertBefore(newNode, refNode) {
      if (!refNode) return this.appendChild(newNode);
      const i = this.children.indexOf(refNode);
      if (i < 0) return this.appendChild(newNode);
      this.children.splice(i, 0, newNode);
      this.childNodes.splice(i, 0, newNode);
      newNode.parentNode = this;
      return newNode;
    }
    contains(node) {
      if (!node) return false;
      return this === node || this.children.some((c) => c?.contains?.(node));
    }
  }

  class MinimalDocument extends MinimalEventTarget {
    constructor() {
      super();
      this.nodeType = 9;
    }
    createElement(tagName) {
      return new MinimalNode(tagName);
    }
    createTextNode(value) {
      const n = new MinimalNode("#text");
      n.nodeValue = value;
      n.nodeType = 3;
      return n;
    }
    createComment(value) {
      const n = new MinimalNode("#comment");
      n.nodeValue = value;
      n.nodeType = 8;
      return n;
    }
    get body() {
      if (!this._body) this._body = this.createElement("body");
      return this._body;
    }
    get activeElement() {
      return null;
    }
    contains(node) {
      return node != null;
    }
  }

  globalThis.document = new MinimalDocument();
  globalThis.HTMLIFrameElement = MinimalNode;
  globalThis.HTMLElement = MinimalNode;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";
  if (typeof globalThis.window === "undefined") {
    Object.defineProperty(globalThis, "window", {
      value: globalThis,
      configurable: true,
    });
  }
  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 16);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
}

installDOMShim();
const storageValues = new Map();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: {
    get length() {
      return storageValues.size;
    },
    key: (index) => [...storageValues.keys()][index] ?? null,
    getItem: (key) => storageValues.get(key) ?? null,
    setItem: (key, value) => storageValues.set(key, String(value)),
    removeItem: (key) => storageValues.delete(key),
    clear: () => storageValues.clear(),
  },
});

import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { WorkspacesProvider } from "@/features/workspaces/useWorkspaces";
import {
  channelMessagesQueryOptions,
  useChannelMessagesQuery,
  useChannelSubscription,
} from "./hooks.ts";
import {
  registerSubscriptionIntent,
  resetChannelOpenGateForTests,
} from "./lib/channelOpenGate.ts";
import {
  channelMessagesKey,
  channelWindowKey,
} from "./lib/messageQueryKeys.ts";
import { relayClient } from "@/shared/api/relayClient";

const CHANNEL = {
  id: "chan-open",
  name: "Open",
  channelType: "stream",
  visibility: "open",
  description: "",
  topic: null,
  purpose: null,
  memberCount: 1,
  memberPubkeys: [],
  lastMessageAt: null,
  archivedAt: null,
  participants: [],
  participantPubkeys: [],
  isMember: true,
  ttlSeconds: null,
  ttlDeadline: null,
};
const pause = () => {
  let resolve;
  const promise = new Promise((r) => {
    resolve = r;
  });
  return { promise, resolve };
};
const event = (id, created_at) => ({
  id,
  pubkey: "a".repeat(64),
  created_at,
  kind: 9,
  tags: [["h", CHANNEL.id]],
  content: id,
  sig: "",
});
const response = (id, at) => [
  event(id, at),
  {
    id: `bounds-${id}`,
    pubkey: "b".repeat(64),
    created_at: at,
    kind: 39006,
    tags: [["d", `${CHANNEL.id}:head`]],
    content: JSON.stringify({ has_more: false, next_cursor: null }),
    sig: "",
  },
];
function Harness() {
  useChannelSubscription(CHANNEL);
  useChannelMessagesQuery(CHANNEL);
  return null;
}
async function mount(client) {
  const root = createRoot(document.createElement("div"));
  await act(async () =>
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          WorkspacesProvider,
          null,
          React.createElement(Harness),
        ),
      ),
    ),
  );
  return async () => {
    await act(async () => root.unmount());
    client.clear();
  };
}
const eventually = async (predicate) => {
  for (let i = 0; i < 50 && !predicate(); i++)
    await new Promise((r) => setTimeout(r, 0));
  assert.ok(predicate());
};
let subscribe, reconnects;
test.beforeEach(() => {
  resetChannelOpenGateForTests();
  subscribe = relayClient.subscribeToChannelLive;
  reconnects = relayClient.subscribeToReconnects;
  relayClient.subscribeToReconnects = () => () => {};
});
test.afterEach(() => {
  relayClient.subscribeToChannelLive = subscribe;
  relayClient.subscribeToReconnects = reconnects;
  clearMocks();
});

test("mount gates head transport until subscription activation, then starts exactly one", async () => {
  const active = pause();
  relayClient.subscribeToChannelLive = async () => {
    await active.promise;
    return async () => {};
  };
  let calls = 0;
  mockIPC((cmd) => {
    assert.equal(cmd, "get_channel_window");
    calls++;
    return response("ordered", 2);
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const unmount = await mount(client);
  assert.equal(calls, 0);
  active.resolve();
  await eventually(() => calls === 1);
  await client.ensureQueryData({
    ...channelMessagesQueryOptions(client, CHANNEL),
    staleTime: Infinity,
  });
  assert.equal(calls, 1);
  await unmount();
});

test("activation joins cold unordered prefetch then launches exactly one ordered fetch", async () => {
  const first = pause(),
    active = pause();
  let calls = 0;
  mockIPC((cmd) => {
    assert.equal(cmd, "get_channel_window");
    calls++;
    return calls === 1 ? first.promise : response("ordered", 2);
  });
  relayClient.subscribeToChannelLive = async () => {
    await active.promise;
    return async () => {};
  };
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const prefetch = client.prefetchQuery(
    channelMessagesQueryOptions(client, CHANNEL),
  );
  await eventually(() => calls === 1);
  const unmount = await mount(client);
  active.resolve();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(calls, 1);
  first.resolve(response("unordered", 1));
  await prefetch;
  await eventually(() => calls === 2);
  await client.ensureQueryData({
    ...channelMessagesQueryOptions(client, CHANNEL),
    staleTime: Infinity,
  });
  assert.equal(
    client.getQueryData(channelMessagesKey(CHANNEL.id))[0].id,
    "ordered",
  );
  await unmount();
});

test("stale non-abortable transport finishing last cannot overwrite ordered query or window", async () => {
  const stale = pause();
  let calls = 0;
  mockIPC((cmd) => {
    assert.equal(cmd, "get_channel_window");
    calls++;
    return calls === 1 ? stale.promise : response("ordered", 2);
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const options = channelMessagesQueryOptions(client, CHANNEL);
  const old = client.fetchQuery(options).catch(() => {});
  await eventually(() => calls === 1);
  await client.cancelQueries({ queryKey: options.queryKey });
  const intent = registerSubscriptionIntent(CHANNEL.id);
  assert.equal(intent.activate(), true);
  await client.fetchQuery({ ...options, staleTime: 0 });
  assert.equal(calls, 2);
  stale.resolve(response("stale", 1));
  await old;
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(
    client.getQueryData(channelMessagesKey(CHANNEL.id))[0].id,
    "ordered",
  );
  assert.equal(
    client.getQueryData(channelWindowKey(CHANNEL.id)).pages[0].rows[0].event.id,
    "ordered",
  );
  intent.dispose();
  client.clear();
});
