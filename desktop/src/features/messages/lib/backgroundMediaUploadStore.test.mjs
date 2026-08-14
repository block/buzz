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
  const startedProgressIds = new Set();
  const cancelled = [];
  const dispatched = [];
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
});
