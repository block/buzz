import assert from "node:assert/strict";
import test from "node:test";

import { applyImetaUpdate, compactImetaSlots } from "./imetaSlots.ts";

// An immediate upload reserves a `null` placeholder and later fills it *by
// index*. Updates written against the compacted list must therefore be mapped
// back onto the slot layout: replacing the array renumbers it under an
// in-flight upload, whose fillSlot would then overwrite an unrelated
// attachment.

const SNAPSHOT = { url: "snapshot.png", sha256: "5555" };
const append = (descriptor) => (current) => [...current, descriptor];

test("compaction hides in-flight placeholders", () => {
  const only = { url: "only.png", sha256: "1111" };
  assert.deepEqual(compactImetaSlots([null, only, null]), [only]);
  assert.deepEqual(compactImetaSlots([]), []);
});

test("a snapshot paste during an in-flight upload does not take its slot", () => {
  // Repro: attach a photo (slot 0 reserved, still uploading), then paste an
  // agent snapshot. The snapshot must land after the placeholder so the
  // photo's fillSlot(0, ...) cannot overwrite it.
  const slots = applyImetaUpdate([null], append(SNAPSHOT));
  assert.deepEqual(slots, [null, SNAPSHOT]);

  // The upload completes and fills its own reserved index.
  const photo = { url: "photo.png", sha256: "aaaa" };
  const filled = [...slots];
  filled[0] = photo;
  assert.deepEqual(filled, [photo, SNAPSHOT]);
});

test("an append keeps already-filled attachments at their own indexes", () => {
  const first = { url: "first.png", sha256: "1111" };
  assert.deepEqual(applyImetaUpdate([first, null], append(SNAPSHOT)), [
    first,
    null,
    SNAPSHOT,
  ]);
});

test("an updater returning its input leaves the slots untouched", () => {
  // handleSnapshotPaste returns `current` unchanged when the snapshot is
  // already attached; that must not disturb a reserved placeholder.
  const existing = [SNAPSHOT, null];
  const slots = applyImetaUpdate(existing, (current) => current);
  assert.equal(slots, existing, "same array identity, no re-render churn");
});

test("a removal nulls its slot instead of renumbering", () => {
  // Removing an attachment must not shift the index a pending upload holds.
  const keep = { url: "keep.png", sha256: "1111" };
  const drop = { url: "drop.png", sha256: "2222" };
  const slots = applyImetaUpdate([keep, drop, null], (current) =>
    current.filter((d) => d.url !== "drop.png"),
  );
  assert.deepEqual(slots, [keep, null, null]);
});

test("clearing every attachment keeps the reserved placeholders", () => {
  const one = { url: "one.png", sha256: "1111" };
  assert.deepEqual(
    applyImetaUpdate([one, null], () => []),
    [null, null],
  );
});

test("the updater only ever sees real attachments", () => {
  const only = { url: "only.png", sha256: "1111" };
  let seen = null;
  applyImetaUpdate([null, only, null], (current) => {
    seen = current;
    return current;
  });
  assert.deepEqual(seen, [only]);
});

test("descriptors are matched on url and digest together", () => {
  // Same url, different bytes: the new descriptor is an append, not a survivor.
  const original = { url: "same.png", sha256: "1111" };
  const reuploaded = { url: "same.png", sha256: "2222" };
  assert.deepEqual(applyImetaUpdate([original, null], append(reuploaded)), [
    original,
    null,
    reuploaded,
  ]);
});

test("a reorder does not move descriptors out of their slots", () => {
  // Reordering cannot be honored while an upload holds an index; keeping the
  // existing positions is what protects the pending fillSlot.
  const a = { url: "a.png", sha256: "1111" };
  const b = { url: "b.png", sha256: "2222" };
  const slots = applyImetaUpdate([a, b, null], (current) =>
    [...current].reverse(),
  );
  assert.deepEqual(slots, [a, b, null]);
});
