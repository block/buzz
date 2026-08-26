import assert from "node:assert/strict";
import test from "node:test";

import {
  cancelStartedMediaUploads,
  dispatchTrackedMediaUpload,
} from "./backgroundMediaUploadStore.ts";

const descriptor = {
  url: "https://relay.example/media/file.bin",
  sha256: "a".repeat(64),
  size: 1,
  type: "application/octet-stream",
  uploaded: 0,
};

function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test("cancels only uploads whose native commands were dispatched", async () => {
  const releaseUpload = deferred();
  const startedProgressIds = new Map();
  const cancelled = [];
  const dispatched = [];
  const released = [];
  const upload = async (_file, id, _signal, onDispatch) => {
    dispatched.push(id);
    onDispatch();
    await releaseUpload.promise;
    return descriptor;
  };
  const ids = Array.from({ length: 129 }, (_, index) => `attachment-${index}`);

  const uploadPromise = dispatchTrackedMediaUpload(
    {},
    ids[0],
    new AbortController().signal,
    startedProgressIds,
    upload,
    async (id) => released.push(id),
  );
  await Promise.resolve();

  cancelStartedMediaUploads(startedProgressIds, async (id) => {
    cancelled.push(id);
  });

  assert.deepEqual(dispatched, [ids[0]]);
  assert.deepEqual(cancelled, [ids[0]]);
  assert.equal(startedProgressIds.size, 1);

  releaseUpload.resolve();
  await uploadPromise;
  assert.equal(startedProgressIds.size, 0);
  assert.deepEqual(released, [ids[0]]);
});

test("releases ownership when dispatch rejects", async () => {
  const startedProgressIds = new Map();
  const released = [];

  await assert.rejects(
    dispatchTrackedMediaUpload(
      {},
      "rejected",
      new AbortController().signal,
      startedProgressIds,
      async (_file, id, _signal, onDispatch) => {
        onDispatch();
        throw new Error(`rejected ${id}`);
      },
      async (id) => released.push(id),
    ),
    /rejected rejected/,
  );

  assert.equal(startedProgressIds.size, 0);
  assert.deepEqual(released, ["rejected"]);
});

test("waits for cancellation before releasing renderer ownership", async () => {
  const releaseUpload = deferred();
  const releaseCancellation = deferred();
  const startedProgressIds = new Map();
  const events = [];
  const uploadPromise = dispatchTrackedMediaUpload(
    {},
    "ordered",
    new AbortController().signal,
    startedProgressIds,
    async (_file, _id, _signal, onDispatch) => {
      onDispatch();
      await releaseUpload.promise;
      return descriptor;
    },
    async () => events.push("release"),
  );
  await Promise.resolve();

  cancelStartedMediaUploads(startedProgressIds, async () => {
    events.push("cancel-start");
    await releaseCancellation.promise;
    events.push("cancel-finish");
  });
  releaseUpload.resolve();
  await Promise.resolve();
  assert.deepEqual(events, ["cancel-start"]);

  releaseCancellation.resolve();
  await uploadPromise;
  assert.deepEqual(events, ["cancel-start", "cancel-finish", "release"]);
});
