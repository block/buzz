import assert from "node:assert/strict";
import { test } from "node:test";
import { finalizeEvent, generateSecretKey } from "nostr-tools/pure";
import { EnterpriseMediaCredentials } from "./enterprise-media.ts";

const key = generateSecretKey();
function harness() {
  let now = 1000;
  const events = [];
  const signer = {
    signEvent: async (e) => {
      events.push(e);
      return finalizeEvent(e, key);
    },
  };
  return {
    cache: new EnterpriseMediaCredentials(
      signer,
      "https://buzz.example",
      () => now,
    ),
    events,
    advance: (seconds) => {
      now += seconds;
    },
  };
}

test("media read credentials are single-flight, host-scoped and expire", async () => {
  const { cache, events, advance } = harness();
  const [first, second] = await Promise.all([
    cache.read("https://buzz.example/a"),
    cache.read("https://buzz.example/b"),
  ]);
  assert.equal(first, second);
  assert.equal(events.length, 1);
  assert.equal(await cache.read("https://buzz.example/c"), first);
  await assert.rejects(cache.read("https://evil.example/a"), /outside/);
  await assert.rejects(cache.read("https://buzz.example:444/a"), /outside/);
  await assert.rejects(cache.read("https://user@buzz.example/a"), /outside/);
  advance(106);
  assert.notEqual(await cache.read("https://buzz.example/a"), first);
  assert.equal(events.length, 2);
});

test("clearing credentials fences in-flight reads and failures remain retryable", async () => {
  let finish;
  let fail = true;
  const signer = {
    signEvent: (e) =>
      fail
        ? Promise.reject(Error("login required"))
        : new Promise((resolve) => {
            finish = () => resolve(finalizeEvent(e, key));
          }),
  };
  const cache = new EnterpriseMediaCredentials(
    signer,
    "https://buzz.example",
    () => 1000,
  );
  await assert.rejects(cache.read("https://buzz.example/a"), /login/);
  fail = false;
  const pending = cache.read("https://buzz.example/a");
  cache.clear();
  finish();
  await assert.rejects(pending, /identity changed/);
});

test("upload sends a file-bound hash to signer, not bytes", async () => {
  const { cache, events } = harness();
  const bytes = new TextEncoder().encode("abc").buffer;
  const header = await cache.upload("https://buzz.example/upload", bytes);
  assert.ok(header.startsWith("Nostr "));
  assert.deepEqual(
    events[0].tags.find((t) => t[0] === "x"),
    ["x", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"],
  );
  assert.equal(events[0].content, "Upload buzz-media");
  assert.equal(events[0].tags.find((t) => t[0] === "expiration")[1], "1120");
});
