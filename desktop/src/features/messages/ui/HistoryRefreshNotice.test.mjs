import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useChannelMessagesQuery, useSendMessageMutation } from "../hooks.ts";
import { HistoryRefreshNotice } from "./HistoryRefreshNotice.tsx";
import { relayClient } from "../../../shared/api/relayClient.ts";
import {
  channelMessagesKey,
  channelWindowKey,
} from "../lib/messageQueryKeys.ts";
import { parseChannelWindowResponse } from "../lib/channelWindowResponse.ts";
import {
  appendOlderChannelWindow,
  emptyChannelWindowStore,
  replaceNewestChannelWindow,
} from "../lib/channelWindowStore.ts";
import {
  projectChannelWindowMessages,
  refreshChannelWindowMessages,
} from "../lib/projectChannelWindow.ts";

const channel = { id: "channel", channelType: "stream" };
const event = (index) => ({
  id: index.toString(16).padStart(64, "0"),
  pubkey: "a".repeat(64),
  created_at: 5000 - index,
  kind: 9,
  tags: [["h", channel.id]],
  content: `row ${index}`,
  sig: "",
});
const server = Array.from({ length: 150 }, (_, i) => event(i));
function wirePage(cursor, limit = 50) {
  const start = cursor
    ? server.findIndex((e) => e.id === cursor.event_id) + 1
    : 0;
  const rows = server.slice(start, start + limit);
  const more = start + rows.length < server.length;
  const last = rows.at(-1);
  return [
    ...rows,
    {
      ...event(999),
      kind: 39006,
      tags: [
        [
          "d",
          cursor
            ? `channel:${cursor.created_at}:${cursor.event_id}`
            : "channel:head",
        ],
      ],
      content: JSON.stringify({
        has_more: more,
        next_cursor: more ? { created_at: last.created_at, id: last.id } : null,
      }),
    },
  ];
}
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
async function until(predicate) {
  for (let i = 0; i < 100 && !predicate(); i++) await act(tick);
  assert.ok(predicate(), "expected async state to settle");
}
async function setup(t, retry = false) {
  const dom = new JSDOM("<div id='root'></div>", { url: "http://localhost" });
  const globals = {
    window: dom.window,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    IS_REACT_ACT_ENVIRONMENT: true,
  };
  const saved = Object.fromEntries(
    Object.keys(globals).map((key) => [
      key,
      Object.getOwnPropertyDescriptor(globalThis, key),
    ]),
  );
  for (const [key, value] of Object.entries(globals))
    Object.defineProperty(globalThis, key, {
      value,
      configurable: true,
      writable: true,
    });
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry, retryDelay: 0, gcTime: Infinity },
      mutations: { retry: false, gcTime: Infinity },
    },
  });
  let store = emptyChannelWindowStore();
  for (let i = 0; i < 3; i++) {
    const cursor = store.pages.at(-1)?.nextCursor ?? null;
    const wireCursor = cursor
      ? { created_at: cursor.createdAt, event_id: cursor.eventId }
      : null;
    const page = parseChannelWindowResponse(
      wirePage(wireCursor),
      channel.id,
      cursor,
    );
    store = i
      ? appendOlderChannelWindow(store, page)
      : replaceNewestChannelWindow(store, page);
  }
  client.setQueryData(channelWindowKey(channel.id), {
    ...store,
    refreshError:
      "Couldn’t refresh messages. Your loaded history is still available.",
  });
  projectChannelWindowMessages(client, channel.id);
  const requests = [];
  let respond = async () => {};
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      assert.equal(command, "get_channel_window");
      requests.push(args.cursor ? "page" : "head");
      const response = wirePage(args.cursor, args.limitRows);
      await respond(args);
      return response;
    },
  };
  t.mock.method(relayClient, "sendMessage", async () => ({
    ...event(1000),
    created_at: 6000,
    content: "sent while refreshing",
  }));
  let mutation;
  let navigations = 0;
  function Screen({ active }) {
    useChannelMessagesQuery(active ? channel : null);
    mutation = useSendMessageMutation(channel, { pubkey: "a".repeat(64) });
    return React.createElement(HistoryRefreshNotice, {
      channelId: active ? channel.id : null,
      onLoadLatest: () => navigations++,
    });
  }
  const root = createRoot(document.getElementById("root"));
  const render = async (active = true) =>
    act(async () =>
      root.render(
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(Screen, { active }),
        ),
      ),
    );
  await render();
  t.after(async () => {
    await act(async () => {
      root.unmount();
      client.clear();
      await tick();
    });
    dom.window.close();
    for (const [key, descriptor] of Object.entries(saved)) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  });
  const read = () => client.getQueryData(channelWindowKey(channel.id));
  return {
    client,
    requests,
    read,
    render,
    navigations: () => navigations,
    respond: (fn) => {
      respond = fn;
    },
    send: () =>
      act(async () => {
        await mutation.mutateAsync({ content: "sent while refreshing" });
      }),
    click: async (label = "Load latest") => {
      const button = [...document.querySelectorAll("button")].find(
        (b) => b.textContent === label,
      );
      assert.ok(button);
      assert.equal(button.disabled, false);
      await act(async () => button.click());
    },
    refresh: () =>
      act(async () => {
        await refreshChannelWindowMessages(client, channel.id);
      }),
    idle: () =>
      until(
        () =>
          client.isFetching({ queryKey: channelMessagesKey(channel.id) }) === 0,
      ),
    rows: () => read().pages.reduce((n, page) => n + page.rows.length, 0),
  };
}

for (const cancel of ["send", "navigation"]) {
  test(`Load latest canceled by ${cancel} cannot discard history on a later implicit refresh`, async (t) => {
    const h = await setup(t);
    let release;
    h.respond(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    await h.click();
    await until(() => !!release);
    if (cancel === "send") await h.send();
    else await h.render(false);
    await h.idle();
    assert.equal(h.rows(), 150);
    await act(async () => {
      release();
      await tick();
    });
    h.respond(async () => {});
    if (cancel === "navigation") await h.render();
    await h.refresh();
    assert.equal(
      h.rows(),
      150,
      "unrelated refresh must not spend canceled navigation intent",
    );
    assert.equal(h.navigations(), 0);
  });
}

test("successful Load latest replaces retained pages even when the head is unchanged", async (t) => {
  const h = await setup(t);
  await h.click();
  await h.idle();
  assert.equal(h.rows(), 50);
  assert.equal(h.read().pages.length, 1);
  assert.equal(h.read().refreshError, undefined);
  assert.equal(h.navigations(), 1);
  assert.deepEqual(h.requests, ["head"]);
});

test("Load latest keeps its intent across real TanStack retry: 1 attempts", async (t) => {
  const h = await setup(t, 1);
  let attempts = 0;
  h.respond(async () => {
    if (++attempts === 1) throw Error("first attempt failed");
  });
  await h.click();
  await h.idle();
  assert.equal(h.rows(), 50);
  assert.equal(h.read().pages.length, 1);
  assert.equal(h.navigations(), 1);
  assert.deepEqual(h.requests, ["head", "head"]);
});

for (const label of ["Retry", "Load latest"]) {
  test(`${label} exhausted retries preserve history and never navigate`, async (t) => {
    const h = await setup(t, 1);
    h.respond(async () => {
      throw Error("offline");
    });
    await h.click(label);
    await h.idle();
    assert.equal(h.rows(), 150);
    assert.ok(h.read().refreshError);
    assert.equal(h.navigations(), 0);
    assert.deepEqual(h.requests, ["head", "head"]);
    h.respond(async () => {});
    await h.refresh();
    assert.equal(h.rows(), 150);
    assert.equal(h.navigations(), 0);
  });
}

test("unmounting before invalidation starts cannot leave an unclaimed Load latest token", async (t) => {
  const h = await setup(t);
  const originalInvalidate = h.client.invalidateQueries.bind(h.client);
  let release;
  t.mock.method(h.client, "invalidateQueries", async (...args) => {
    await new Promise((resolve) => {
      release = resolve;
    });
    return originalInvalidate(...args);
  });
  await h.click();
  await until(() => !!release);
  await h.render(false);
  await act(async () => {
    release();
    await tick();
  });
  assert.equal(h.read().refreshLatestOnly, undefined);
  assert.equal(h.navigations(), 0);
  assert.deepEqual(h.requests, []);
});
