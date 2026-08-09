import assert from "node:assert/strict";
import test from "node:test";

import {
  addForcedUnreadSource,
  forcedUnreadMarker,
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

test("legacy persisted entries retain their read-marker baseline", () => {
  assert.equal(forcedUnreadMarker(120), 120);
  assert.equal(forcedUnreadMarker(null), null);
});
