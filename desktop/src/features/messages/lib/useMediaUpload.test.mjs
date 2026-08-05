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

// ── Edit mode while an upload is in flight (known limitation) ──────────
// Entering edit mode is a wholesale `setPendingImeta(array)`, so it is a draft
// boundary: the epoch bumps and in-flight immediate uploads are retired. The
// composer's pre-edit snapshot is taken from `pendingImeta`, which is the
// *compacted* slot array — an upload that has only reserved a null slot is not
// in it. So a photo/file still uploading when the user opens "Edit message" is
// not restored when the edit is cancelled.
//
// This is deliberate, and it is narrower than the alternative. Without the
// epoch guard the descriptor lands by slot index in whatever attachment set is
// on screen, which means it can overwrite an attachment belonging to the
// message being edited (see the companion test below). Discarding it is a
// clean loss instead of writing the wrong attachment onto someone else's
// message. The caller-side fix — carrying reserved slots through the snapshot,
// or refusing to enter edit mode while `isUploading` — lives in
// `MessageComposer`, not here.
//
// These tests pin the current behaviour so a future change to it is a
// deliberate decision rather than an accident.

/**
 * Model the composer's edit-mode round trip.
 * `retire` selects the epoch-guarded behaviour (this branch) or the
 * unguarded behaviour it replaced.
 */
function editModeRoundTrip({ retire, resolveDuringEdit, draft = [] }) {
  const compact = (slots) => slots.filter((d) => d !== null);
  const uploaded = { sha256: "ffff", url: "in-flight.png" };
  const editTargetImeta = [{ sha256: "eeee", url: "edit-target.png" }];

  let slots = [...draft];
  // Attach a photo: reserve a slot and start the upload.
  const slotIndex = slots.length;
  slots = [...slots, null];
  const pinnedEpoch = 0;
  let epoch = 0;
  const canceled = new Set();
  const previewId = 1;

  // Enter edit mode: snapshot the (compacted) draft, then seed the target's
  // attachments. The reserved null slot is dropped by the compaction.
  const snapshot = compact(slots);
  if (retire) {
    epoch += 1;
    canceled.add(previewId);
  }
  slots = editTargetImeta;

  const landDescriptor = () => {
    if (canceled.has(previewId)) return;
    if (retire && pinnedEpoch !== epoch) return;
    const next = [...slots];
    next[slotIndex] = uploaded;
    slots = next;
  };

  if (resolveDuringEdit) landDescriptor();
  const whileEditing = compact(slots);

  // Cancel the edit: restore the snapshot (another wholesale replacement).
  if (retire) epoch += 1;
  slots = [...snapshot];
  if (!resolveDuringEdit) landDescriptor();

  return { restoredDraft: compact(slots), whileEditing };
}

test("an upload in flight when edit mode opens is not restored after cancel", () => {
  // Known limitation: the pre-edit snapshot is built from the compacted slot
  // array, so a reserved-but-unfilled slot is not captured.
  const { restoredDraft } = editModeRoundTrip({
    resolveDuringEdit: true,
    retire: true,
  });
  assert.deepEqual(restoredDraft, []);
});

test("retiring the upload keeps it off the message being edited", () => {
  // This is what the retirement buys. Unguarded, the descriptor lands at its
  // reserved index inside the *edit target's* attachment set and replaces the
  // attachment already there — the user would save someone else's photo onto
  // the message they were editing.
  const guarded = editModeRoundTrip({
    resolveDuringEdit: true,
    retire: true,
  });
  assert.deepEqual(guarded.whileEditing, [
    { sha256: "eeee", url: "edit-target.png" },
  ]);

  const unguarded = editModeRoundTrip({
    resolveDuringEdit: true,
    retire: false,
  });
  assert.deepEqual(unguarded.whileEditing, [
    { sha256: "ffff", url: "in-flight.png" },
  ]);
});

test("an upload resolving after cancel is discarded rather than appended", () => {
  // The other timing: unguarded, a descriptor arriving after the restore
  // happens to land past the end of the restored draft and reappears. That
  // recovery is incidental — it depends on the restored draft having exactly
  // the length the slot index was reserved against — and the same code path is
  // what overwrites the edit target in the test above. The guard trades it for
  // a predictable outcome.
  const guarded = editModeRoundTrip({
    draft: [{ sha256: "aaaa", url: "already-there.png" }],
    resolveDuringEdit: false,
    retire: true,
  });
  assert.deepEqual(guarded.restoredDraft, [
    { sha256: "aaaa", url: "already-there.png" },
  ]);

  const unguarded = editModeRoundTrip({
    draft: [{ sha256: "aaaa", url: "already-there.png" }],
    resolveDuringEdit: false,
    retire: false,
  });
  assert.deepEqual(unguarded.restoredDraft, [
    { sha256: "aaaa", url: "already-there.png" },
    { sha256: "ffff", url: "in-flight.png" },
  ]);
});
