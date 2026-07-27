/**
 * Convergence regression test for useLiveChannelUpdates.
 *
 * One event can match BOTH per-channel live subscriptions at once: the general
 * `#h` subscription and the `#p` mention subscription (HOME_MENTION_EVENT_KINDS
 * ⊆ CHANNEL_EVENT_KINDS). The relay fans out one frame per subscription, so a
 * message that p-tags the reader *and* carries a NIP-CM notify tag arrives
 * twice on the same connection and both frames land in handleIncomingMessage.
 * Without a shared seen-set there, onChannelMessage fires twice and the reader
 * gets two identical OS notifications with two mention sounds.
 *
 * These tests mount the REAL hook against stubbed relayClient subscriptions and
 * a real QueryClientProvider, capture the two subscription callbacks, and drive
 * the same event through both. They fail if the dedup guard at the top of
 * handleIncomingMessage is removed.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Minimal DOM shim (react-dom/client needs a document) ─────────────────────

function installDOMShim() {
  class EventTargetShim {
    constructor() {
      this.listeners = new Map();
    }

    addEventListener(type, listener) {
      this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
    }

    removeEventListener(type, listener) {
      this.listeners.set(
        type,
        (this.listeners.get(type) ?? []).filter(
          (current) => current !== listener,
        ),
      );
    }

    dispatchEvent(event) {
      for (const listener of this.listeners.get(event.type) ?? [])
        listener(event);
      return true;
    }
  }

  class NodeShim extends EventTargetShim {
    constructor(tagName) {
      super();
      this.tagName = tagName;
      this.nodeName = tagName.toUpperCase();
      this.nodeType = 1;
      this.namespaceURI = "http://www.w3.org/1999/xhtml";
      this.children = [];
      this.childNodes = [];
      this.style = {};
      this.parentNode = null;
    }

    get ownerDocument() {
      return globalThis.document;
    }

    get firstChild() {
      return this.children[0] ?? null;
    }

    get lastChild() {
      return this.children.at(-1) ?? null;
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
      this.children = this.children.filter((current) => current !== child);
      this.childNodes = this.childNodes.filter((current) => current !== child);
      child.parentNode = null;
      return child;
    }

    insertBefore(child, reference) {
      if (!reference) return this.appendChild(child);
      const index = this.children.indexOf(reference);
      if (index < 0) return this.appendChild(child);
      this.children.splice(index, 0, child);
      this.childNodes.splice(index, 0, child);
      child.parentNode = this;
      return child;
    }

    contains(node) {
      return (
        this === node || this.children.some((child) => child.contains(node))
      );
    }
  }

  class DocumentShim extends EventTargetShim {
    constructor() {
      super();
      this.nodeType = 9;
      this.defaultView = globalThis;
    }

    createElement(tagName) {
      return new NodeShim(tagName);
    }

    createTextNode(value) {
      const node = new NodeShim("#text");
      node.nodeType = 3;
      node.nodeValue = value;
      return node;
    }

    createComment(value) {
      const node = new NodeShim("#comment");
      node.nodeType = 8;
      node.nodeValue = value;
      return node;
    }

    get activeElement() {
      return null;
    }
  }

  globalThis.document = new DocumentShim();
  const windowEvents = new EventTargetShim();
  globalThis.addEventListener =
    windowEvents.addEventListener.bind(windowEvents);
  globalThis.removeEventListener =
    windowEvents.removeEventListener.bind(windowEvents);
  globalThis.dispatchEvent = windowEvents.dispatchEvent.bind(windowEvents);
  globalThis.HTMLElement = NodeShim;
  globalThis.HTMLIFrameElement = NodeShim;
  globalThis.Node = NodeShim;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: globalThis,
  });
  globalThis.localStorage = {
    getItem: () => null,
    removeItem: () => {},
    setItem: () => {},
  };
  globalThis.requestAnimationFrame = (callback) => setTimeout(callback, 0);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
}

installDOMShim();

// ── Production imports (after the shim) ──────────────────────────────────────

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";
import { useLiveChannelUpdates } from "./useLiveChannelUpdates.ts";

const CHANNEL_ID = "11111111-1111-4111-8111-111111111111";
const READER = "b".repeat(64);
const AUTHOR = "a".repeat(64);

const channels = [
  { id: CHANNEL_ID, name: "general", channelType: "stream", createdAt: 0 },
];

/** A message that both p-tags the reader and carries an @here marker. */
function mentionAndNotifyEvent(id) {
  return {
    id,
    kind: KIND_STREAM_MESSAGE,
    pubkey: AUTHOR,
    created_at: Math.floor(Date.now() / 1000),
    content: "@reader @here ship it",
    sig: "s".repeat(128),
    tags: [
      ["h", CHANNEL_ID],
      ["p", READER],
      ["notify", "here"],
    ],
  };
}

/**
 * Mount the hook with stubbed relay subscriptions. Returns the captured
 * subscription callbacks plus the callback invocation logs.
 */
async function mountHook() {
  const generalCallbacks = [];
  const mentionCallbacks = [];
  const channelMessages = [];
  const liveMentions = [];

  const noopDispose = async () => {};
  relayClient.subscribeToReconnects = () => () => {};
  relayClient.subscribeLive = async (_filter, onEvent) => {
    generalCallbacks.push(onEvent);
    return noopDispose;
  };
  relayClient.subscribeToChannelMentionEvents = async (
    _channelId,
    _pubkey,
    onEvent,
  ) => {
    mentionCallbacks.push(onEvent);
    return noopDispose;
  };

  function Harness() {
    useLiveChannelUpdates(channels, null, {
      currentPubkey: READER,
      onChannelMessage: (channelId, event) =>
        channelMessages.push([channelId, event.id]),
      onLiveMention: () => liveMentions.push(true),
    });
    return null;
  }

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(Harness),
      ),
    );
  });

  assert.equal(generalCallbacks.length, 1, "general subscription installed");
  assert.equal(mentionCallbacks.length, 1, "mention subscription installed");

  return {
    channelMessages,
    deliverGeneral: generalCallbacks[0],
    deliverMention: mentionCallbacks[0],
    liveMentions,
    unmount: async () => {
      await act(async () => root.unmount());
      queryClient.clear();
    },
  };
}

test("both subscriptions delivering one event notify the channel once", async () => {
  const harness = await mountHook();
  const event = mentionAndNotifyEvent("e".repeat(64));

  await act(async () => {
    harness.deliverGeneral(event);
    harness.deliverMention(event);
  });

  assert.deepEqual(harness.channelMessages, [[CHANNEL_ID, event.id]]);
  assert.equal(harness.liveMentions.length, 1);

  await harness.unmount();
});

test("mention-first delivery order also notifies once", async () => {
  const harness = await mountHook();
  const event = mentionAndNotifyEvent("f".repeat(64));

  await act(async () => {
    harness.deliverMention(event);
    harness.deliverGeneral(event);
  });

  assert.deepEqual(harness.channelMessages, [[CHANNEL_ID, event.id]]);
  assert.equal(harness.liveMentions.length, 1);

  await harness.unmount();
});

test("distinct events are not swallowed by the dedup set", async () => {
  const harness = await mountHook();
  const first = mentionAndNotifyEvent("1".repeat(64));
  const second = mentionAndNotifyEvent("2".repeat(64));

  await act(async () => {
    harness.deliverGeneral(first);
    harness.deliverMention(first);
    harness.deliverGeneral(second);
  });

  assert.deepEqual(harness.channelMessages, [
    [CHANNEL_ID, first.id],
    [CHANNEL_ID, second.id],
  ]);
  assert.equal(harness.liveMentions.length, 1);

  await harness.unmount();
});
