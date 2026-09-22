import assert from "node:assert/strict";
import test from "node:test";

import {
  addForcedUnreadSource,
  boundForcedUnreadMap,
  forcedUnreadMarker,
  MAX_FORCED_UNREAD_ENTRIES,
  removeForcedUnreadSource,
} from "./forcedUnreadStore.ts";

test("adding an Inbox owner preserves an existing manual force", () => {
  const entry = addForcedUnreadSource(null, 120, "inbox");

  assert.deepEqual(entry, {
    markerAtWhenForced: null,
    sources: ["manual", "inbox"],
  });
});

test("clearing Inbox ownership leaves a manual force intact", () => {
  const entry = {
    markerAtWhenForced: 120,
    sources: ["manual", "inbox"],
  };

  assert.deepEqual(removeForcedUnreadSource(entry, "inbox"), {
    markerAtWhenForced: 120,
    sources: ["manual"],
  });
});

test("clearing manual ownership leaves an Inbox force intact", () => {
  const entry = {
    markerAtWhenForced: 120,
    sources: ["manual", "inbox"],
  };

  assert.deepEqual(removeForcedUnreadSource(entry, "manual"), {
    markerAtWhenForced: 120,
    sources: ["inbox"],
  });
});

test("clearing the only force owner removes the entry", () => {
  const entry = {
    markerAtWhenForced: 120,
    sources: ["inbox"],
  };

  assert.equal(removeForcedUnreadSource(entry, "inbox"), undefined);
});

test("forced unread map keeps the newest 500 insertion-ordered entries", () => {
  const map = Object.fromEntries(
    Array.from({ length: MAX_FORCED_UNREAD_ENTRIES + 2 }, (_, index) => [
      `channel-${index}`,
      index,
    ]),
  );

  const bounded = boundForcedUnreadMap(map);

  assert.equal(Object.keys(bounded).length, MAX_FORCED_UNREAD_ENTRIES);
  assert.equal(bounded["channel-0"], undefined);
  assert.equal(bounded["channel-1"], undefined);
  assert.equal(bounded[`channel-${MAX_FORCED_UNREAD_ENTRIES + 1}`], 501);
});

test("legacy persisted entries retain their read-marker baseline", () => {
  assert.equal(forcedUnreadMarker(120), 120);
  assert.equal(forcedUnreadMarker(null), null);
});
