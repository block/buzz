import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { refreshDesktopList } from "./desktopList.ts";
import { DesktopListView } from "./ui/KnownDesktops.tsx";

const scope = { owner: "owner-a", community: "wss://a.example" };
const first = {
  id: "signed-a",
  pubkey: scope.owner,
  kind: 30180,
  tags: [["d", "desktop-a"]],
};
const second = { ...first, id: "signed-b", tags: [["d", "desktop-b"]] };
function fixture() {
  const events = new Map([[second.id, second]]);
  const calls = [];
  let epoch = 0;
  const ipc = async (command, args) => {
    assert.equal(args.owner, scope.owner);
    assert.equal(args.community, scope.community);
    calls.push(command);
    if (command === "prepare_desktop_profile") return { event: first };
    if (command === "read_desktop_profiles")
      return args.events.map((event) => ({
        id: event.tags[0][1],
        name: event.tags[0][1],
        updated: 100,
      }));
  };
  const relay = {
    getSessionEpoch: () => epoch,
    fetchEvents: async (filter) => {
      assert.deepEqual(filter.authors, [scope.owner]);
      assert.deepEqual(filter.kinds, [30180]);
      assert.ok(filter.limit <= 100);
      return [...events.values()].filter(
        (event) => !filter["#d"] || filter["#d"].includes(event.tags[0][1]),
      );
    },
    publishEvent: async (event) => {
      calls.push(event);
      events.set(event.id, event);
    },
  };
  return {
    ipc,
    relay,
    calls,
    events,
    switchScope: () => {
      epoch++;
    },
  };
}

test("two same-owner Desktops retained without presence; startup doesn't rewrite", async () => {
  const f = fixture();
  const list = await refreshDesktopList(scope, () => true, f.ipc, f.relay);
  assert.equal(list.rows.length, 2);
  assert.equal(list.local, "desktop-a");
  assert.equal(list.warning, "");
  assert.deepEqual(
    await refreshDesktopList(scope, () => true, f.ipc, f.relay),
    list,
  );
  assert.equal(f.calls.filter((call) => call === first).length, 1);
});

test("ACK loss retries the exact object, while disconnected entries remain listed", async () => {
  const f = fixture();
  f.relay.publishEvent = async (event) => {
    f.calls.push(event);
    throw Error("lost ACK");
  };
  for (let retry = 0; retry < 2; retry++) {
    const list = await refreshDesktopList(scope, () => true, f.ipc, f.relay);
    assert.equal(list.rows.length, 1);
    assert.ok(list.warning);
  }
  assert.deepEqual(
    f.calls.filter((call) => typeof call === "object"),
    [first, first],
  );
  assert.ok(!f.calls.includes("acknowledge_desktop_profile"));
});

test("owner/community switch fences prepared bytes and late ACKs", async () => {
  for (const boundary of ["prepare_desktop_profile", "publish"]) {
    const f = fixture();
    const ipc = async (command, args) => {
      const result = await f.ipc(command, args);
      if (command === boundary) f.switchScope();
      return result;
    };
    if (boundary === "publish")
      f.relay.publishEvent = async () => f.switchScope();
    await assert.rejects(
      refreshDesktopList(scope, () => true, ipc, f.relay),
      /scope changed/,
    );
    assert.ok(!f.calls.includes("acknowledge_desktop_profile"));
    assert.ok(!f.calls.includes(first));
  }
});

test("denied coordinate read is not absence, invalid reader result is not an empty list", async () => {
  const f = fixture();
  const fetch = f.relay.fetchEvents;
  f.relay.fetchEvents = async (filter) => {
    if (filter["#d"]) throw Error("denied");
    return fetch(filter);
  };
  assert.ok(
    (await refreshDesktopList(scope, () => true, f.ipc, f.relay)).warning,
  );
  assert.ok(!f.calls.includes(first));
  await assert.rejects(
    refreshDesktopList(
      scope,
      () => true,
      async () => {
        throw Error("invalid");
      },
      f.relay,
    ),
  );
});

test("rendered list distinguishes current, partial, unavailable and empty without online claims", () => {
  const list = {
    rows: [{ id: "a", name: "Desktop a", updated: 100 }],
    local: "a",
    warning: "",
    partial: true,
  };
  const render = (data, error = false) =>
    renderToStaticMarkup(
      React.createElement(DesktopListView, {
        list: data,
        error,
        loading: false,
        refresh() {},
      }),
    );
  const html = render(list, true);
  assert.match(html, /This Desktop/);
  assert.match(html, /Profile updated/);
  assert.match(html, /Partial list/);
  assert.match(html, /unavailable/);
  assert.match(html, /Desktop a/);
  assert.doesNotMatch(html, /No Desktop profiles found/);
  assert.match(
    render({ ...list, rows: [], partial: false }),
    /No Desktop profiles found/,
  );
  assert.match(html, /Last heard: Unknown/);
  assert.doesNotMatch(html, /Online|Offline/);
});

test("mounted cache clears both scopes, fences late reads and retains rows on failure", async (t) => {
  const { JSDOM } = await import("jsdom");
  const dom = new JSDOM("<div id='root'></div>", {
    url: "https://desktop.test",
  });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const { createRoot } = await import("react-dom/client");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { CommunitiesProvider, useCommunities } = await import(
    "../communities/useCommunities.tsx"
  );
  const { KnownDesktops, DesktopListStartup } = await import(
    "./ui/KnownDesktops.tsx"
  );
  const { relayClient } = await import("../../shared/api/relayClient.ts");
  const communities = ["a", "b"].map((id) => ({
    id,
    name: id,
    relayUrl: `wss://${id}.example`,
  }));
  localStorage.setItem("buzz-communities", JSON.stringify(communities));
  localStorage.setItem("buzz-active-community-id", "a");
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  client.setQueryData(["identity"], { pubkey: "owner-a" });
  let controls;
  let fail = false;
  let hold = false;
  let release;
  let current;
  const originalFetch = relayClient.fetchEvents;
  const originalPublish = relayClient.publishEvent;
  const originalReconnect = relayClient.subscribeToReconnects;
  let reconnect;
  let pulses = 0;
  let reports = 0;
  let publishedReports = 0;
  let deferReport = true;
  relayClient.subscribeToReconnects = (callback) => {
    reconnect = callback;
    return () => {
      reconnect = undefined;
    };
  };
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      if (command === "prepare_desktop_capabilities") {
        reports++;
        if (deferReport) throw Error("clock has not advanced");
        return { event: { ...first, kind: 30182 } };
      }
      if (command === "read_desktop_capabilities")
        return args.events.map(() => ({
          id: "desktop-a",
          reported: 100,
          runtimes: [],
        }));
      if (command === "prepare_desktop_observation")
        return { event: { ...first, kind: 30181 } };
      if (command === "read_desktop_observations")
        return [
          {
            id: `${args.owner}-${args.community}`,
            heard: Math.floor(Date.now() / 1000),
          },
        ];
      if (command === "prepare_desktop_profile") {
        current = {
          ...first,
          id: `${args.owner}-${args.community}`,
          tags: [["d", args.owner]],
        };
        return { event: current };
      }
      assert.equal(command, "read_desktop_profiles");
      return args.events.map((event) => ({
        id: event.id,
        name: event.id,
        updated: 100,
      }));
    },
  };
  relayClient.fetchEvents = async (filter) => {
    if (fail) throw Error("unavailable");
    if ([30181, 30182].includes(filter.kinds[0])) return [];
    if (filter["#d"]) return [current];
    const rows = [current];
    if (hold) {
      hold = false;
      return new Promise((resolve) => {
        release = () => resolve(rows);
      });
    }
    return rows;
  };
  relayClient.publishEvent = async (event) => {
    if (event.kind === 30182) {
      publishedReports++;
      return;
    }
    assert.equal(event.kind, 30181, "no profile heartbeat rewrite");
    pulses++;
  };
  function Screen() {
    controls = useCommunities();
    return React.createElement(
      React.Fragment,
      null,
      React.createElement(DesktopListStartup),
      React.createElement(KnownDesktops),
    );
  }
  const root = createRoot(document.getElementById("root"));
  const settle = () =>
    React.act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
  const text = () => document.body.textContent;
  t.mock.timers.enable({ apis: ["setInterval"] });
  try {
    await React.act(async () =>
      root.render(
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(
            CommunitiesProvider,
            null,
            React.createElement(Screen),
          ),
        ),
      ),
    );
    await settle();
    assert.match(text(), /owner-a-wss:\/\/a.example/);
    assert.match(text(), /Last heard: Recent/);
    assert.match(text(), /could not synchronize capability facts/);
    const beforeReconnect = pulses;
    const reportsBeforeReconnect = reports;
    await React.act(async () => reconnect());
    await settle();
    assert.ok(pulses > beforeReconnect, "reconnect reports a fresh pulse");
    assert.ok(
      reports > reportsBeforeReconnect,
      "reconnect retries deferred facts",
    );
    const beforeTimer = pulses;
    const reportsBeforeTimer = reports;
    await React.act(async () => t.mock.timers.tick(60_000));
    await settle();
    assert.ok(pulses > beforeTimer, "bounded periodic publisher runs");
    assert.ok(reports > reportsBeforeTimer, "periodic retry survives deferral");
    assert.equal(publishedReports, 0, "deferred facts are not published");
    deferReport = false;
    await React.act(async () => t.mock.timers.tick(60_000));
    await settle();
    assert.equal(publishedReports, 1, "later preparation is published");
    assert.doesNotMatch(text(), /could not synchronize capability facts/);
    hold = true;
    await React.act(async () => {
      void client.refetchQueries({ queryKey: ["desktop-profiles"] });
    });
    assert.equal(typeof release, "function");
    await React.act(async () => {
      client.setQueryData(["identity"], { pubkey: "owner-b" });
    });
    await settle();
    assert.doesNotMatch(text(), /owner-a/);
    await React.act(async () => release());
    await settle();
    assert.match(text(), /owner-b-wss:\/\/a.example/);
    assert.doesNotMatch(text(), /owner-a/);
    assert.equal(
      client.getQueryData(["desktop-profiles", "owner-a", "wss://a.example"]),
      undefined,
    );
    fail = true;
    await React.act(async () => {
      await client.refetchQueries({ queryKey: ["desktop-profiles"] });
      await client.refetchQueries({ queryKey: ["desktop-observations"] });
    });
    await settle();
    assert.match(text(), /Last-heard refresh unavailable/);
    assert.match(text(), /Last heard: Recent/);
    assert.match(text(), /unavailable/);
    assert.match(text(), /owner-b-wss:\/\/a.example/);
    await React.act(async () => controls.switchCommunity("b"));
    await settle();
    assert.doesNotMatch(text(), /owner-b-wss:\/\/a.example/);
    fail = false;
    await React.act(async () => {
      await client.refetchQueries({ queryKey: ["desktop-profiles"] });
      await client.refetchQueries({ queryKey: ["desktop-observations"] });
    });
    await settle();
    assert.match(text(), /owner-b-wss:\/\/b.example/);
  } finally {
    await React.act(async () => root.unmount());
    client.clear();
    relayClient.fetchEvents = originalFetch;
    relayClient.publishEvent = originalPublish;
    relayClient.subscribeToReconnects = originalReconnect;
    assert.equal(
      reconnect,
      undefined,
      "reconnect producer unsubscribed on unmount",
    );
    t.mock.timers.reset();
    dom.window.close();
  }
});
