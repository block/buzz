import assert from "node:assert/strict";
import test from "node:test";
import { getOrCreateDeviceId } from "./businessAuthGateway.ts";

test("device id is stable and contains no identity secret", () => {
  const values = new Map();
  const storage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  };
  const first = getOrCreateDeviceId(storage);
  assert.equal(first, getOrCreateDeviceId(storage));
  assert.match(first, /^[0-9a-f-]{36}$/);
});
