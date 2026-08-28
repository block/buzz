import assert from "node:assert/strict";
import test from "node:test";

import { commitGuardedNavigation } from "./commitGuardedNavigation.ts";

const route = (href) => ({ kind: "route", href });

test("an accepted navigation consults the guard before navigating", async () => {
  const order = [];
  const committed = await commitGuardedNavigation(
    {
      currentHref: "/channels/aaaa",
      nextHref: "/channels/bbbb",
      guardedTarget: route("/channels/bbbb"),
      navigate: async () => {
        order.push("navigate");
      },
    },
    {
      allow: () => {
        order.push("guard");
        return true;
      },
    },
  );
  assert.equal(committed, true);
  assert.deepEqual(order, ["guard", "navigate"]);
});

test("a same-destination no-op never reaches the guard", async () => {
  const order = [];
  const committed = await commitGuardedNavigation(
    {
      currentHref: "/channels/aaaa",
      nextHref: "/channels/aaaa",
      guardedTarget: route("/channels/aaaa"),
      navigate: async () => {
        order.push("navigate");
      },
    },
    {
      allow: () => {
        order.push("guard");
        return true;
      },
    },
  );
  assert.equal(committed, false);
  assert.deepEqual(order, []);
});

test("force overrides the same-destination no-op but still runs the guard first", async () => {
  const order = [];
  const committed = await commitGuardedNavigation(
    {
      currentHref: "/channels/aaaa",
      nextHref: "/channels/aaaa",
      force: true,
      guardedTarget: route("/channels/aaaa"),
      navigate: async () => {
        order.push("navigate");
      },
    },
    {
      allow: () => {
        order.push("guard");
        return true;
      },
    },
  );
  assert.equal(committed, true);
  assert.deepEqual(order, ["guard", "navigate"]);
});

test("a same-destination navigation carrying router state still commits", async () => {
  const order = [];
  const committed = await commitGuardedNavigation(
    {
      currentHref: "/channels/aaaa",
      nextHref: "/channels/aaaa",
      guardedTarget: route("/channels/aaaa"),
      hasStateUpdate: true,
      navigate: async () => {
        order.push("navigate");
      },
    },
    {
      allow: () => {
        order.push("guard");
        return true;
      },
    },
  );
  assert.equal(committed, true);
  assert.deepEqual(order, ["guard", "navigate"]);
});

test("a refused navigation does not navigate", async () => {
  // The guard's whole purpose: an unsaved thread edit blocks the switch.
  // Asserting only that the guard is CONSULTED is not enough — consulting it
  // and discarding the answer passes every ordering test in this file.
  const order = [];
  const committed = await commitGuardedNavigation(
    {
      currentHref: "/channels/aaaa",
      nextHref: "/channels/bbbb",
      guardedTarget: route("/channels/bbbb"),
      navigate: async () => {
        order.push("navigate");
      },
    },
    {
      allow: () => {
        order.push("guard");
        return false;
      },
    },
  );
  assert.equal(committed, false);
  assert.deepEqual(order, ["guard"]);
});

test("a refused forced navigation does not navigate either", async () => {
  // force defeats the same-destination skip, never the guard.
  const order = [];
  const committed = await commitGuardedNavigation(
    {
      currentHref: "/channels/aaaa",
      nextHref: "/channels/aaaa",
      force: true,
      guardedTarget: route("/channels/aaaa"),
      navigate: async () => {
        order.push("navigate");
      },
    },
    { allow: () => false },
  );
  assert.equal(committed, false);
  assert.deepEqual(order, []);
});
