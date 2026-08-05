import assert from "node:assert/strict";
import test from "node:test";

// shortHash is a simple utility: str.slice(0, 4)
// Inline it here to avoid importing from useMediaUpload.ts which has
// unresolvable @/shared path aliases outside the bundler.
function shortHash(hex) {
  return hex.slice(0, 4);
}

// ── shortHash ─────────────────────────────────────────────────────────

test("shortHash returns first 4 hex characters", () => {
  assert.equal(shortHash("abcdef1234567890"), "abcd");
});

test("shortHash handles minimum-length input", () => {
  assert.equal(shortHash("abcd"), "abcd");
});

test("shortHash returns empty string for empty input", () => {
  assert.equal(shortHash(""), "");
});

test("shortHash returns partial for short input", () => {
  assert.equal(shortHash("ab"), "ab");
});

// ── Upload slot ordering (pure state-update logic) ────────────────────
// The slot system uses reserveSlots → fillSlot to maintain insertion order
// when concurrent uploads finish out of order. We test the state-update
// functions in isolation (they're pure array transforms).

test("reserveSlots creates null placeholders", () => {
  // Simulate: start with empty slots, reserve 3
  const prev = [];
  const count = 3;
  const next = [...prev, ...new Array(count).fill(null)];
  assert.deepEqual(next, [null, null, null]);
});

test("fillSlot places descriptor at correct index", () => {
  // Simulate: 3 reserved slots, fill index 1 first (out of order)
  const slots = [null, null, null];
  const descriptor = { url: "https://example.com/b.png", sha256: "bbbb" };
  const next = [...slots];
  next[1] = descriptor;
  assert.equal(next[0], null);
  assert.deepEqual(next[1], descriptor);
  assert.equal(next[2], null);
});

test("concurrent uploads filling out of order preserves slot positions", () => {
  // Simulate: reserve 3 slots, uploads finish in order 2, 0, 1
  const slots = [null, null, null];
  const a = { url: "a.png", sha256: "aaaa" };
  const b = { url: "b.png", sha256: "bbbb" };
  const c = { url: "c.png", sha256: "cccc" };

  // Upload 2 finishes first
  const step1 = [...slots];
  step1[2] = c;
  assert.deepEqual(step1, [null, null, c]);

  // Upload 0 finishes second
  const step2 = [...step1];
  step2[0] = a;
  assert.deepEqual(step2, [a, null, c]);

  // Upload 1 finishes last
  const step3 = [...step2];
  step3[1] = b;
  assert.deepEqual(step3, [a, b, c]);

  // Filter nulls — final order matches original slot order
  const result = step3.filter((d) => d !== null);
  assert.deepEqual(result, [a, b, c]);
});

test("removing an attachment nulls the slot instead of compacting", () => {
  const a = { url: "a.png", sha256: "aaaa" };
  const b = { url: "b.png", sha256: "bbbb" };
  const c = { url: "c.png", sha256: "cccc" };
  const slots = [a, b, c];

  // Remove b — null out, don't compact
  const next = slots.map((d) => (d?.url === "b.png" ? null : d));
  assert.deepEqual(next, [a, null, c]);
  // Filtered view (what consumers see) drops nulls
  const filtered = next.filter((d) => d !== null);
  assert.deepEqual(filtered, [a, c]);
});

test("removing mid-upload does not corrupt in-flight slot indices", () => {
  // Scenario: 3 images uploading, image 0 finishes, user removes image 0,
  // then image 1 and 2 finish — they must land in their original slots.
  const a = { url: "a.png", sha256: "aaaa" };
  const b = { url: "b.png", sha256: "bbbb" };
  const c = { url: "c.png", sha256: "cccc" };

  // Start: 3 reserved slots
  let slots = [null, null, null];

  // Image 0 finishes
  slots = [...slots];
  slots[0] = a;
  assert.deepEqual(slots, [a, null, null]);

  // User removes image 0 — null out, don't compact
  slots = slots.map((d) => (d?.url === "a.png" ? null : d));
  assert.deepEqual(slots, [null, null, null]);

  // Image 1 finishes — fillSlot(1) still works correctly
  slots = [...slots];
  slots[1] = b;
  assert.deepEqual(slots, [null, b, null]);

  // Image 2 finishes — fillSlot(2) still works correctly
  slots = [...slots];
  slots[2] = c;
  assert.deepEqual(slots, [null, b, c]);

  // Consumer view filters nulls
  const result = slots.filter((d) => d !== null);
  assert.deepEqual(result, [b, c]);
});

test("reserveSlots pads if slots array is shorter than expected start index", () => {
  // Edge case: if somehow prev is shorter than startIndex
  const prev = [{ url: "a.png", sha256: "aaaa" }];
  const startIndex = 3;
  const count = 2;
  const padded =
    prev.length < startIndex
      ? [...prev, ...new Array(startIndex - prev.length).fill(null)]
      : prev;
  const next = [...padded, ...new Array(count).fill(null)];
  assert.equal(next.length, 5);
  assert.deepEqual(next[0], { url: "a.png", sha256: "aaaa" });
  assert.equal(next[1], null); // padding
  assert.equal(next[2], null); // padding
  assert.equal(next[3], null); // reserved
  assert.equal(next[4], null); // reserved
});

// ── Draft-boundary epoch guard (pure logic) ───────────────────────────
// Photos/files upload immediately, so an upload can still be in flight when
// the composer swaps drafts (channel switch, post-send clear, edit restore).
// Every wholesale `setPendingImeta` replacement bumps an epoch; uploads pin
// the epoch at start and discard their descriptor if it no longer matches, so
// one draft's attachment can never land in — or overwrite a slot reserved by —
// another draft. Mirrors `isUploadStale` + `fillSlot`/`onUploaded`.

function fillSlotIfCurrent(slots, index, descriptor, epoch, currentEpoch) {
  if (epoch !== currentEpoch) return slots;
  const next = [...slots];
  next[index] = descriptor;
  return next;
}

test("upload completing in the same draft fills its slot", () => {
  const a = { url: "a.png", sha256: "aaaa" };
  const next = fillSlotIfCurrent([null], 0, a, 0, 0);
  assert.deepEqual(next, [a]);
});

test("upload completing after a draft switch is discarded", () => {
  // Draft A reserves slot 0 at epoch 0, user switches channels (epoch → 1),
  // then the upload resolves. It must not write into draft B's slots.
  const a = { url: "a.png", sha256: "aaaa" };
  const draftBSlots = [null];
  const next = fillSlotIfCurrent(draftBSlots, 0, a, 0, 1);
  assert.deepEqual(next, [null]);
  assert.equal(next, draftBSlots);
});

test("stale upload cannot overwrite a slot the new draft already filled", () => {
  // Draft B has its own attachment in slot 0; draft A's late upload targets
  // the same index and must leave B's descriptor intact.
  const stale = { url: "stale.png", sha256: "aaaa" };
  const current = { url: "current.png", sha256: "bbbb" };
  const next = fillSlotIfCurrent([current], 0, stale, 0, 2);
  assert.deepEqual(next, [current]);
});

test("appending to the current draft does not bump the epoch", () => {
  // Only wholesale replacement (`setPendingImeta(array)`) is a draft boundary.
  // The updater form appends within the current draft, so in-flight uploads
  // for that same draft must still be considered current.
  let epoch = 0;
  const bumpIfReplacement = (action) => {
    if (typeof action !== "function") epoch += 1;
  };
  bumpIfReplacement((current) => [...current, { url: "pasted.png" }]);
  assert.equal(epoch, 0);
  bumpIfReplacement([]);
  assert.equal(epoch, 1);
});

// ── Cancel guard for stale previews (pure logic) ───────────────────────
// The epoch bump makes completions discard their descriptors, but the old
// preview row (and its cancel button) can still be on screen. Cancelling it
// must not null a slot in the draft now on screen, because the preview carries
// the *previous* draft's slotIndex. Mirrors `cancelUpload`'s `isStalePreview`.

function cancelSlotIndex(preview, currentEpoch) {
  if (preview?.slotIndex === undefined) return undefined;
  const isStale =
    preview.uploadEpoch !== undefined && preview.uploadEpoch !== currentEpoch;
  return isStale ? undefined : preview.slotIndex;
}

test("cancelling a preview from the current draft nulls its slot", () => {
  assert.equal(cancelSlotIndex({ slotIndex: 1, uploadEpoch: 3 }, 3), 1);
});

test("cancelling a stale preview does not null the new draft's slot", () => {
  // Draft A reserved slot 0 at epoch 0; draft B now owns slot 0. Cancelling
  // A's leftover preview must leave B's attachment intact.
  assert.equal(cancelSlotIndex({ slotIndex: 0, uploadEpoch: 0 }, 1), undefined);
});

test("cancelling a preview with no slot is a no-op for slots", () => {
  // `handlePaperclip`'s native-picker preview has no reserved slot.
  assert.equal(cancelSlotIndex({ uploadEpoch: 0 }, 0), undefined);
});

// ── Retiring in-flight uploads at a draft boundary (pure logic) ────────
// Bumping the epoch alone discards descriptors but leaves the previous draft's
// preview rows on screen and its uploads counted, which keeps `isUploading`
// true and holds the *new* draft's send gate closed. A wholesale replacement
// must therefore retire those uploads outright. Mirrors `beginNewDraftEpoch`.

function beginNewDraftEpoch(state) {
  const next = {
    epoch: state.epoch + 1,
    active: new Set(state.active),
    canceled: new Set(state.canceled),
    previews: state.previews,
    uploadingCount: state.uploadingCount,
  };
  if (next.active.size === 0) return next;
  // Mirrors the real callback: snapshot, clear the live set, then schedule the
  // updaters. `applyUpdates` below runs them afterwards, the way React does.
  const retiredIds = new Set(next.active);
  const retiredCount = retiredIds.size;
  next.active.clear();
  for (const id of retiredIds) next.canceled.add(id);
  next.pendingUpdates = [
    (s) => {
      s.previews = s.previews.filter((preview) => !retiredIds.has(preview.id));
    },
    (s) => {
      s.uploadingCount = Math.max(0, s.uploadingCount - retiredCount);
    },
  ];
  return next;
}

/** Run the scheduled state updaters, as React does after the event handler. */
function applyUpdates(state) {
  for (const update of state.pendingUpdates ?? []) update(state);
  state.pendingUpdates = [];
  return state;
}

test("a draft boundary retires in-flight uploads so the new draft can send", () => {
  // Draft A has one upload in flight; switching to draft B must leave B with
  // no previews and nothing counted as uploading.
  const after = applyUpdates(
    beginNewDraftEpoch({
      epoch: 0,
      active: new Set([1]),
      canceled: new Set(),
      previews: [{ id: 1, slotIndex: 0, uploadEpoch: 0 }],
      uploadingCount: 1,
    }),
  );
  assert.equal(after.epoch, 1);
  assert.deepEqual(after.previews, []);
  assert.equal(after.uploadingCount, 0);
  assert.equal(after.active.size, 0);
  // Canceled so the late completion/error paths stay silent in the new draft.
  assert.ok(after.canceled.has(1));
});

test("retiring several concurrent uploads clears the count exactly once each", () => {
  const after = applyUpdates(
    beginNewDraftEpoch({
      epoch: 4,
      active: new Set([7, 8, 9]),
      canceled: new Set(),
      previews: [{ id: 7 }, { id: 8 }, { id: 9 }],
      uploadingCount: 3,
    }),
  );
  assert.equal(after.uploadingCount, 0);
  assert.deepEqual(after.previews, []);
});

test("a draft boundary with no uploads in flight still advances the epoch", () => {
  const after = applyUpdates(
    beginNewDraftEpoch({
      epoch: 2,
      active: new Set(),
      canceled: new Set(),
      previews: [],
      uploadingCount: 0,
    }),
  );
  assert.equal(after.epoch, 3);
  assert.equal(after.uploadingCount, 0);
});

test("the retired count never drives uploadingCount negative", () => {
  // Defensive: a preview already settled by finishUpload must not be
  // double-decremented into a negative count that would wedge the gate.
  const after = applyUpdates(
    beginNewDraftEpoch({
      epoch: 0,
      active: new Set([1, 2]),
      canceled: new Set(),
      previews: [{ id: 1 }, { id: 2 }],
      uploadingCount: 1,
    }),
  );
  assert.equal(after.uploadingCount, 0);
});

test("retirement holds even though the live active set is cleared first", () => {
  // Regression: the updaters must not read the live `active` set, which is
  // emptied before React runs them. Closing over it filtered against an empty
  // set and subtracted 0, leaving the stale preview and a stuck send gate.
  const state = beginNewDraftEpoch({
    epoch: 0,
    active: new Set([1]),
    canceled: new Set(),
    previews: [{ id: 1 }],
    uploadingCount: 1,
  });
  assert.equal(state.active.size, 0, "live set is cleared before updates run");
  // Updates land only now — after the clear — exactly as React schedules them.
  applyUpdates(state);
  assert.deepEqual(state.previews, []);
  assert.equal(state.uploadingCount, 0);
});

test("replayed updaters stay idempotent", () => {
  // React may invoke an updater more than once (StrictMode double-render).
  const state = beginNewDraftEpoch({
    epoch: 0,
    active: new Set([1]),
    canceled: new Set(),
    previews: [{ id: 1 }],
    uploadingCount: 1,
  });
  const updates = state.pendingUpdates;
  for (const update of updates) update(state);
  for (const update of updates) update(state);
  assert.deepEqual(state.previews, []);
  assert.equal(state.uploadingCount, 0);
});

// ── Edit mode while an upload is in flight ────────────────────────────
// Immediate photo/file uploads reserve null slots that are absent from the
// compacted `pendingImeta` snapshot. MessageComposer therefore rejects edit
// entry while an upload is active, leaving the current draft and upload epoch
// untouched. Once the upload settles, normal edit snapshot/restore proceeds.

function attemptEditModeRoundTrip({ isUploading, draft = [] }) {
  const uploaded = { sha256: "ffff", url: "in-flight.png" };
  const editTargetImeta = [{ sha256: "eeee", url: "edit-target.png" }];
  let slots = [...draft];
  let epoch = 0;

  if (isUploading) {
    slots = [...slots, null];
    return {
      editEntered: false,
      epoch,
      restoredDraft: slots,
    };
  }

  const snapshot = [...slots];
  epoch += 1;
  slots = editTargetImeta;
  epoch += 1;
  slots = snapshot;

  return {
    editEntered: true,
    epoch,
    restoredDraft: slots,
    uploaded,
  };
}

test("edit entry is rejected without replacing a draft that is uploading", () => {
  const existing = { sha256: "aaaa", url: "already-there.png" };
  const result = attemptEditModeRoundTrip({
    draft: [existing],
    isUploading: true,
  });

  assert.equal(result.editEntered, false);
  assert.equal(result.epoch, 0, "the current draft epoch must not be retired");
  assert.deepEqual(result.restoredDraft, [existing, null]);
});

test("edit entry proceeds normally after uploads settle", () => {
  const existing = { sha256: "aaaa", url: "already-there.png" };
  const result = attemptEditModeRoundTrip({
    draft: [existing],
    isUploading: false,
  });

  assert.equal(result.editEntered, true);
  assert.equal(result.epoch, 2);
  assert.deepEqual(result.restoredDraft, [existing]);
});
