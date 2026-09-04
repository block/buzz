import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { refreshDesktopCapabilities } from "./desktopCapabilities.ts";
import { DesktopListView } from "./ui/KnownDesktops.tsx";

const scope = { owner: "owner-a", community: "wss://one.example" };
const event = { id: "signed", created_at: 100, kind: 30182 };
const row = {
  id: "desktop-a",
  reported: 100,
  runtimes: [
    {
      id: "goose",
      availability: "cli_missing",
      requires_external_cli: true,
      max_parallelism: null,
    },
  ],
};
function fixture(boundary) {
  let epoch = 0;
  const calls = [];
  const finish = (name, result) => {
    if (name === boundary) epoch++;
    return result;
  };
  const f = {
    calls,
    head: [],
    ipc: async (command, args) => {
      assert.equal(args.owner, scope.owner);
      assert.equal(args.community, scope.community);
      calls.push(command);
      if (command === "prepare_desktop_capabilities")
        return finish("prepare", { event });
      assert.equal(command, "read_desktop_capabilities");
      return finish(
        "read",
        args.events.map(() => row),
      );
    },
    relay: {
      getSessionEpoch: () => epoch,
      fetchEvents: async (filter) => {
        assert.deepEqual(filter.authors, [scope.owner]);
        assert.deepEqual(filter.kinds, [30182]);
        assert.ok(filter.limit <= 100);
        return finish("fetch", filter["#d"] ? f.head : [event]);
      },
      publishEvent: async (value, _timeout, _failure, check) => {
        finish("transport");
        check(); // Delayed transport must invoke the production cancellation guard.
        calls.push(value);
        f.head = [value];
        finish("ack");
      },
    },
  };
  f.refresh = (active = () => true) =>
    refreshDesktopCapabilities(scope, active, f.ipc, f.relay);
  return f;
}

test("unchanged accepted report does not republish; failed publish retries exact bytes", async () => {
  const f = fixture();
  assert.deepEqual((await f.refresh()).rows, [row]);
  await f.refresh();
  assert.equal(f.calls.filter((c) => c === event).length, 1);
  f.head = [];
  f.relay.publishEvent = async (value) => {
    assert.equal(value, event);
    throw Error("offline");
  };
  for (let i = 0; i < 2; i++) assert.ok((await f.refresh()).warning);
  f.relay.fetchEvents = async () => {
    throw Error("unavailable");
  };
  await assert.rejects(f.refresh(), /unavailable/);
});

test("all async boundaries fence cancellation, account/community switches and late ACK", async () => {
  for (const boundary of ["prepare", "read", "fetch", "transport", "ack"]) {
    const f = fixture(boundary);
    await assert.rejects(f.refresh(), /scope changed/);
    if (boundary !== "ack") assert.ok(!f.calls.includes(event));
  }
  const f = fixture();
  await assert.rejects(f.refresh(() => false));
  assert.deepEqual(f.calls, []);
  f.relay.fetchEvents = async () => Array(100).fill(event);
  assert.equal((await f.refresh()).partial, true);
  f.ipc = async () => {
    throw Error("invalid signature");
  };
  await assert.rejects(f.refresh(), /invalid signature/);
});

test("mounted Desktop rows show exact remote facts and unknowns, not readiness", () => {
  const html = renderToStaticMarkup(
    React.createElement(DesktopListView, {
      list: {
        rows: ["desktop-a", "desktop-b"].map((id) => ({
          id,
          name: id,
          updated: 1,
        })),
        local: "desktop-b",
      },
      capabilities: [row],
      now: 99,
      refresh() {},
      loading: false,
      error: false,
    }),
  );
  assert.equal(
    (html.match(/<summary>Capability details<\/summary>/g) ?? []).length,
    2,
  );
  for (const text of [
    "goose",
    "cli missing",
    "not configured",
    "Desktop clock ahead",
    "No capability report received",
    "not agent readiness",
    "Settings",
  ])
    assert.ok(html.includes(text), text);
});
