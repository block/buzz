import assert from "node:assert/strict";
import test from "node:test";

import { findUnreadNeighbor } from "./unreadConversationOrder.ts";

const displayed = ["starred", "alpha", "beta", "gamma", "dm-1", "dm-2"];
const unread = new Set(["alpha", "gamma", "dm-2"]);
const muted = new Set(["gamma"]);
const none = new Set();

test("next steps over read and muted entries without wrapping", () => {
  assert.equal(
    findUnreadNeighbor(displayed, unread, muted, "starred", "next"),
    "alpha",
  );
  assert.equal(
    findUnreadNeighbor(displayed, unread, muted, "alpha", "next"),
    "dm-2",
  );
  assert.equal(
    findUnreadNeighbor(displayed, unread, muted, "dm-2", "next"),
    null,
  );
});

test("previous steps back without wrapping", () => {
  assert.equal(
    findUnreadNeighbor(displayed, unread, muted, "dm-2", "previous"),
    "alpha",
  );
  assert.equal(
    findUnreadNeighbor(displayed, unread, muted, "alpha", "previous"),
    null,
  );
});

test("an unread current conversation steps past itself", () => {
  assert.equal(
    findUnreadNeighbor(displayed, unread, none, "alpha", "next"),
    "gamma",
  );
  assert.equal(
    findUnreadNeighbor(displayed, unread, none, "gamma", "previous"),
    "alpha",
  );
});

test("a null or unknown current id anchors at the list edge", () => {
  assert.equal(
    findUnreadNeighbor(displayed, unread, muted, null, "next"),
    "alpha",
  );
  assert.equal(
    findUnreadNeighbor(displayed, unread, muted, null, "previous"),
    "dm-2",
  );
  assert.equal(
    findUnreadNeighbor(displayed, unread, muted, "elsewhere", "next"),
    "alpha",
  );
  assert.equal(
    findUnreadNeighbor(displayed, unread, muted, "elsewhere", "previous"),
    "dm-2",
  );
});

test("empty and fully muted projections have no neighbor", () => {
  assert.equal(findUnreadNeighbor([], unread, muted, null, "next"), null);
  assert.equal(
    findUnreadNeighbor(displayed, new Set(), muted, "alpha", "next"),
    null,
  );
  assert.equal(
    findUnreadNeighbor(displayed, unread, new Set(displayed), null, "next"),
    null,
  );
});
