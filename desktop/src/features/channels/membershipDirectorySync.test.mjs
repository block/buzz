import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import {
  refreshDirectoryAfterMembershipChange as refresh,
  resetMembershipDirectorySync,
} from "./membershipDirectorySync.ts";

const KEY = ["relay-agents"];
const clients = [];
const disposers = [];
const settle = () => new Promise((resolve) => setTimeout(resolve, 250));
function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { resolve, promise };
}
function client() {
  const value = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  clients.push(value);
  return value;
}
function observe(value, queryFn, initialData = []) {
  if (initialData !== undefined) value.setQueryData(KEY, initialData);
  const observer = new QueryObserver(value, {
    queryKey: KEY,
    queryFn,
    staleTime: Infinity,
  });
  disposers.push(observer.subscribe(() => {}));
  return observer;
}
afterEach(() => {
  resetMembershipDirectorySync();
  for (const dispose of disposers.splice(0)) dispose();
  for (const value of clients.splice(0)) value.clear();
});

test("membership burst fetches once; replay of the same event does not rebuild the directory", async () => {
  const value = client();
  let calls = 0;
  observe(value, async () => {
    calls += 1;
    return ["current"];
  });
  for (let index = 0; index < 50; index += 1) refresh(value, `event-${index}`);
  assert.equal(value.getQueryState(KEY).isInvalidated, true);
  assert.equal(calls, 0);
  await settle();
  assert.equal(calls, 1);
  for (let index = 0; index < 50; index += 1) refresh(value, `event-${index}`);
  await settle();
  assert.equal(calls, 1);
});

test("a membership change cancels an older read; late positive data cannot overwrite removal", async () => {
  const value = client();
  const old = deferred();
  let calls = 0;
  observe(value, async () => {
    calls += 1;
    return calls === 1 ? old.promise : [];
  }, ["removed-key"]);
  void value.refetchQueries({ queryKey: KEY });
  assert.equal(calls, 1);
  refresh(value, "removed");
  await settle();
  assert.equal(calls, 2);
  assert.deepEqual(value.getQueryData(KEY), []);
  old.resolve(["removed-key"]);
  await settle();
  assert.deepEqual(value.getQueryData(KEY), []);
  assert.equal(calls, 2);
});

test("a cold pending read is also replaced, rather than swallowing the post-write refresh", async () => {
  const value = client();
  const old = deferred();
  let calls = 0;
  const observer = new QueryObserver(value, {
    queryKey: KEY,
    queryFn: () =>
      ++calls === 1 ? old.promise : Promise.resolve(["added-key"]),
  });
  disposers.push(observer.subscribe(() => {}));
  refresh(value, "added");
  await settle();
  assert.equal(calls, 2);
  old.resolve([]);
  await settle();
  assert.deepEqual(value.getQueryData(KEY), ["added-key"]);
});

test("inactive directory is marked stale without starting a relay-wide read", async () => {
  const value = client();
  value.setQueryData(KEY, ["old"]);
  refresh(value);
  await settle();
  assert.equal(value.getQueryState(KEY).isInvalidated, true);
  assert.equal(value.getQueryState(KEY).fetchStatus, "idle");
});

test("failed refresh ends in error and does not schedule a self-sustaining retry loop", async () => {
  const value = client();
  let calls = 0;
  observe(value, async () => {
    calls += 1;
    throw new Error("unavailable");
  });
  refresh(value);
  await settle();
  assert.equal(value.getQueryState(KEY).status, "error");
  await settle();
  assert.equal(calls, 1);
});

test("community reset cancels queued work and event deduplication is client-scoped", async () => {
  const first = client();
  const second = client();
  let firstCalls = 0;
  let secondCalls = 0;
  observe(first, async () => {
    firstCalls += 1;
    return [];
  });
  observe(second, async () => {
    secondCalls += 1;
    return [];
  });
  refresh(first, "shared-event-id");
  resetMembershipDirectorySync();
  refresh(second, "shared-event-id");
  await settle();
  assert.equal(firstCalls, 0);
  assert.equal(secondCalls, 1);
});
