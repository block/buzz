/**
 * Unit tests for `selectSubmitTags` — the pure selector that decides which
 * link-preview snapshot tags a composer submit emits.
 *
 * These import and exercise the ACTUAL source helper (not a mirrored copy), so
 * they fail if the submit-tag selection ever regresses.
 *
 * Regression guard for the "removed-URL tag leak" defect (PR #5245, Blocker B):
 * a ready snapshot tag for URL A lingers in the tag map for the 350 ms
 * debounce window after A is deleted from the draft. Submit must key off the
 * LIVE hrefs in the content being sent — never that debounced set — so deleting
 * A and immediately sending replacement text can never attach A's tag (and its
 * media refs) to a body that no longer contains A.
 *
 * The E2E form of this test was flaky: sending inside the 350 ms window from a
 * headless browser did not reliably fire a submit, so it could not isolate the
 * leak. A pure unit test against the extracted selector is deterministic and
 * targets the fix logic directly.
 */

import assert from "node:assert/strict";
import test from "node:test";
import { selectSubmitTags } from "./useComposerLinkPreviews.tsx";

const tagA = ["link-preview", "snapshot", "1", "https://a.example/x", "A"];
const tagB = ["link-preview", "snapshot", "1", "https://b.example/y", "B"];

test("emits the tag for a live href that has a ready snapshot", () => {
  const tags = selectSubmitTags(
    ["https://a.example/x"],
    { "https://a.example/x": tagA },
    false,
  );
  assert.deepEqual(tags, [tagA]);
});

test("LEAK GUARD: a ready tag whose href is no longer live is NOT emitted", () => {
  // A resolved (tag still cached), but A was deleted from the draft and the
  // live content is now different — the debounced map still holds A's tag.
  const tags = selectSubmitTags(
    [], // live content no longer contains A
    { "https://a.example/x": tagA },
    false,
  );
  assert.deepEqual(tags, [], "removed URL A must never leak its snapshot tag");
});

test("LEAK GUARD: replacing A with a live B emits only B's tag, never A's", () => {
  const tags = selectSubmitTags(
    ["https://b.example/y"], // A deleted, B is what's live now
    { "https://a.example/x": tagA, "https://b.example/y": tagB },
    false,
  );
  assert.deepEqual(tags, [tagB]);
});

test("a live href with no ready tag is omitted (sends as a bare link)", () => {
  const tags = selectSubmitTags(["https://a.example/x"], {}, false);
  assert.deepEqual(tags, []);
});

test("preserves live href order for multiple ready tags", () => {
  const tags = selectSubmitTags(
    ["https://a.example/x", "https://b.example/y"],
    { "https://b.example/y": tagB, "https://a.example/x": tagA },
    false,
  );
  assert.deepEqual(tags, [tagA, tagB]);
});

test("suppressed emits only the 'none' marker, ignoring any ready tags", () => {
  const tags = selectSubmitTags(
    ["https://a.example/x"],
    { "https://a.example/x": tagA },
    true,
  );
  assert.deepEqual(tags, [["link-preview", "none"]]);
});
