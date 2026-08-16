import assert from "node:assert/strict";
import test from "node:test";

import {
  __linkPreviewPreparationTest,
  prepareBackgroundLinkPreviews,
  prepareLinkPreview,
  resetLinkPreviewPreparations,
  skipBackgroundLinkPreviews,
} from "./linkPreviewPreparationStore.ts";

const first = { href: "https://example.com/first" };
const second = { href: "https://example.com/second" };
const firstTag = ["link-preview", "snapshot", first.href];

function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function seed(
  candidate,
  promise,
  settled = false,
  settledAt = Date.now(),
  fallbackTag = null,
  resolvedTag = null,
) {
  __linkPreviewPreparationTest.jobs.set(candidate.href, {
    controller: new AbortController(),
    promise,
    fallbackTag,
    resolvedTag,
    settled,
    settledAt: settled ? settledAt : null,
  });
}

test.afterEach(() => {
  __linkPreviewPreparationTest.reset();
});

test("adopts one in-flight job for the same canonical URL", () => {
  const pending = deferred();
  seed(first, pending.promise);

  assert.equal(prepareLinkPreview(first), pending.promise);
  assert.equal(prepareLinkPreview(first), pending.promise);
  pending.resolve(firstTag);
});

test("expires settled jobs while retaining in-flight and recent work", () => {
  const now = 1_000_000;
  assert.equal(
    __linkPreviewPreparationTest.isReusableJob(
      {
        controller: new AbortController(),
        promise: Promise.resolve(firstTag),
        fallbackTag: null,
        resolvedTag: null,
        settled: false,
        settledAt: null,
      },
      now,
    ),
    true,
  );
  assert.equal(
    __linkPreviewPreparationTest.isReusableJob(
      {
        controller: new AbortController(),
        promise: Promise.resolve(firstTag),
        fallbackTag: null,
        resolvedTag: null,
        settled: true,
        settledAt: now - 1,
      },
      now,
    ),
    true,
  );
  assert.equal(
    __linkPreviewPreparationTest.isReusableJob(
      {
        controller: new AbortController(),
        promise: Promise.resolve(firstTag),
        fallbackTag: null,
        resolvedTag: null,
        settled: true,
        settledAt: now - 5 * 60_000,
      },
      now,
    ),
    false,
  );
});

test("keeps successful sibling tags when another URL fails", async () => {
  const pending = deferred();
  seed(first, Promise.resolve(firstTag), true);
  seed(second, pending.promise);

  const preparation = prepareBackgroundLinkPreviews([first, second], 1_000);
  assert.ok(preparation);
  pending.resolve(null);

  assert.deepEqual(await preparation.promise, {
    status: "ready",
    tags: [firstTag],
  });
});

test("total deadline keeps full and fallback sibling tags", async () => {
  const pending = deferred();
  const fallbackTag = ["link-preview", "snapshot", second.href, "metadata"];
  seed(first, Promise.resolve(firstTag), true, Date.now(), null, firstTag);
  seed(second, pending.promise, false, Date.now(), fallbackTag);

  const preparation = prepareBackgroundLinkPreviews([first, second], 0);
  assert.ok(preparation);
  assert.deepEqual(await preparation.promise, {
    status: "ready",
    tags: [firstTag, fallbackTag],
  });

  const lateTag = ["link-preview", "snapshot", second.href, "image"];
  pending.resolve(lateTag);
  await pending.promise;
  assert.deepEqual(await preparation.promise, {
    status: "ready",
    tags: [firstTag, fallbackTag],
  });
});

test("timeout keeps metadata-only fallback and ignores late upload completion", async () => {
  const pending = deferred();
  const fallbackTag = [
    "link-preview",
    "snapshot",
    "1",
    first.href,
    "First",
    "Example",
    "",
    "",
    "",
    "",
    "",
  ];
  seed(first, pending.promise, false, Date.now(), fallbackTag);

  const preparation = prepareBackgroundLinkPreviews([first], 0);
  assert.ok(preparation);
  assert.deepEqual(await preparation.promise, {
    status: "ready",
    tags: [fallbackTag],
  });

  pending.resolve(firstTag);
  await pending.promise;
  assert.deepEqual(await preparation.promise, {
    status: "ready",
    tags: [fallbackTag],
  });
});

test("Skip wins completion and resolves exactly once", async () => {
  const pending = deferred();
  seed(first, pending.promise);

  const preparation = prepareBackgroundLinkPreviews([first], 1_000);
  assert.ok(preparation);
  preparation.skip();
  pending.resolve(firstTag);

  assert.deepEqual(await preparation.promise, { status: "ready", tags: [] });
});

test("Skip only settles the latest concurrent preparation", async () => {
  const firstPending = deferred();
  const secondPending = deferred();
  seed(first, firstPending.promise);
  seed(second, secondPending.promise);

  const firstPreparation = prepareBackgroundLinkPreviews([first], 1_000);
  const secondPreparation = prepareBackgroundLinkPreviews([second], 1_000);
  assert.ok(firstPreparation);
  assert.ok(secondPreparation);

  skipBackgroundLinkPreviews();
  firstPending.resolve(firstTag);
  secondPending.resolve(["link-preview", "snapshot", second.href]);

  assert.deepEqual(await secondPreparation.promise, {
    status: "ready",
    tags: [],
  });
  assert.deepEqual(await firstPreparation.promise, {
    status: "ready",
    tags: [firstTag],
  });
});

test("Skip after completion cannot replace finalized tags", async () => {
  const pending = deferred();
  seed(first, pending.promise);

  const preparation = prepareBackgroundLinkPreviews([first], 1_000);
  assert.ok(preparation);
  pending.resolve(firstTag);
  assert.deepEqual(await preparation.promise, {
    status: "ready",
    tags: [firstTag],
  });

  preparation.skip();
  assert.deepEqual(await preparation.promise, {
    status: "ready",
    tags: [firstTag],
  });
});

test("already-settled partial results contain only successful tags", async () => {
  seed(first, Promise.resolve(firstTag), true);
  seed(second, Promise.resolve(null), true);

  const preparation = prepareBackgroundLinkPreviews([first, second]);
  assert.ok(preparation);
  assert.deepEqual(await preparation.promise, {
    status: "ready",
    tags: [firstTag],
  });
});

test("reset aborts a promoted send after preview preparation settles", async () => {
  seed(first, Promise.resolve(firstTag), true);

  const preparation = prepareBackgroundLinkPreviews([first]);
  assert.ok(preparation);
  assert.deepEqual(await preparation.promise, {
    status: "ready",
    tags: [firstTag],
  });

  resetLinkPreviewPreparations();
  assert.equal(preparation.signal.aborted, true);
});

test("released promoted sends are no longer cancelled by reset", async () => {
  seed(first, Promise.resolve(firstTag), true);

  const preparation = prepareBackgroundLinkPreviews([first]);
  assert.ok(preparation);
  await preparation.promise;
  preparation.release();

  resetLinkPreviewPreparations();
  assert.equal(preparation.signal.aborted, false);
});

test("reset cancels pending preparations instead of authorizing send", async () => {
  const pending = deferred();
  seed(first, pending.promise);

  const preparation = prepareBackgroundLinkPreviews([first], 1_000);
  assert.ok(preparation);
  resetLinkPreviewPreparations();
  pending.resolve(firstTag);

  assert.deepEqual(await preparation.promise, { status: "cancelled" });
});
