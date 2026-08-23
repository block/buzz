import assert from "node:assert/strict";
import test from "node:test";

import { pickProjectionOverriddenTitle } from "./pickProjectionOverriddenTitle.ts";

const BUMBLE_AIR =
  "aac70a3a960f379f49f26acd28c93f268078591045678e79cff446f913d23e7c";
const POLLEN_AIR =
  "3a2208dba54ebd1fe310052f7bd296a0b4fd50bcdaf38391b9e0cd60e4b068a3";

test("pickProjectionOverriddenTitle — override wins over persona display name for the overridden pubkey", () => {
  const title = pickProjectionOverriddenTitle(
    { [BUMBLE_AIR]: "Bumble Air" },
    BUMBLE_AIR,
    "Pollen",
  );
  assert.equal(title, "Bumble Air");
});

test("pickProjectionOverriddenTitle — pubkey without an override falls through to the fallback", () => {
  const title = pickProjectionOverriddenTitle(
    { [BUMBLE_AIR]: "Bumble Air" },
    POLLEN_AIR,
    "Pollen Air",
  );
  assert.equal(title, "Pollen Air");
});

test("pickProjectionOverriddenTitle — undefined pubkey falls through to the fallback (persona-only card)", () => {
  const title = pickProjectionOverriddenTitle(
    { [BUMBLE_AIR]: "Bumble Air" },
    undefined,
    "Some Persona",
  );
  assert.equal(title, "Some Persona");
});

test("pickProjectionOverriddenTitle — empty overrides map is inert", () => {
  const title = pickProjectionOverriddenTitle({}, BUMBLE_AIR, "raw-name");
  assert.equal(title, "raw-name");
});

test("pickProjectionOverriddenTitle — empty override string does NOT clobber the fallback", () => {
  const title = pickProjectionOverriddenTitle(
    { [BUMBLE_AIR]: "" },
    BUMBLE_AIR,
    "raw-name",
  );
  assert.equal(title, "raw-name");
});
