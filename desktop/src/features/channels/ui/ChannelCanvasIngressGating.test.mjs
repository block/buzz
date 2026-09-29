/**
 * Canvas ingress gating regression: `canOpenCanvas` must key on the presence of
 * a persisted relay revision (`eventId !== null`), not on content length. After
 * a restore-to-empty the relay holds a kind:40100 event with `event_id` set and
 * `content: ""` — a read-only member who cannot edit would lose the only ingress
 * to that revision stream if we gate on `hasCanvas` (content truthiness).
 *
 * Tests the `canvasIngressOpen` pure function that ChannelManagementSheet
 * delegates to for the `canOpenCanvas` flag.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const { canvasIngressOpen } = await import("./canvasIngress.ts");

// Source-wiring oracle: read ChannelManagementSheet.tsx and verify it calls
// `canvasIngressOpen` with `eventId` as the existence signal, not with
// `hasCanvas` (content-length truthiness). Reverting the sheet call site to
// `hasCanvas || canEditNarrative` removes the `canvasIngressOpen` call from
// `canOpenCanvas` and makes the match below fail.
const sheetSource = readFileSync(
  new URL("./ChannelManagementSheet.tsx", import.meta.url),
  "utf8",
).replace(/\s+/g, " ");

test("ChannelManagementSheet: canOpenCanvas is wired to canvasIngressOpen with eventId", () => {
  // The sheet must delegate existence gating to the canonical helper, passing
  // `canvasQuery.data?.eventId` so persisted-empty canvases are not hidden from
  // read-only members. Reverting to `hasCanvas || canEditNarrative` removes
  // this call and the assertion below fails.
  assert.match(
    sheetSource,
    /canvasIngressOpen\( canvasQuery\.data\?\.eventId,/,
    "canOpenCanvas must call canvasIngressOpen(canvasQuery.data?.eventId, …)",
  );
});

test("ChannelManagementSheet: hasCanvas is not used as the ingress-open predicate", () => {
  // `hasCanvas` is a content-length check valid only for preview text. It must
  // NOT be the existence gate for canOpenCanvas.
  assert.doesNotMatch(
    sheetSource,
    /canOpenCanvas = hasCanvas/,
    "canOpenCanvas must not be derived directly from hasCanvas",
  );
});

const EVENT_ID = "a".repeat(64);

// Read-only member (canEditNarrative = false).

test("read-only member: no persisted canvas → ingress closed", () => {
  assert.equal(canvasIngressOpen(null, false), false);
  assert.equal(canvasIngressOpen(undefined, false), false);
});

test("read-only member: persisted canvas with content → ingress open", () => {
  assert.equal(canvasIngressOpen(EVENT_ID, false), true);
});

test("read-only member: persisted empty canvas (content='') with eventId → ingress open", () => {
  // This is the restored-to-empty case. Content is "" but a revision exists.
  // The old `hasCanvas || canEditNarrative` gating would return false here,
  // losing the only ingress to the revision stream for read-only members.
  assert.equal(canvasIngressOpen(EVENT_ID, false), true);
});

// Editor (canEditNarrative = true) — always open regardless of eventId.

test("editor: no persisted canvas → ingress open (seeds first revision)", () => {
  assert.equal(canvasIngressOpen(null, true), true);
  assert.equal(canvasIngressOpen(undefined, true), true);
});

test("editor: persisted canvas → ingress open", () => {
  assert.equal(canvasIngressOpen(EVENT_ID, true), true);
});

// Mutation oracle: confirms the test catches the content-based regression.

test("regression oracle: old content-only gating fails for read-only + persisted-empty", () => {
  // Old code: `hasCanvas || canEditNarrative` where hasCanvas = content.trim().length > 0.
  // For an empty-content revision, hadOldBug === false — ingress closed for read-only.
  const emptyContent = "";
  const hadOldBug = emptyContent.trim().length > 0 || false;
  assert.equal(
    hadOldBug,
    false,
    "old logic closes ingress for read-only + empty content",
  );
  // canvasIngressOpen must NOT replicate that defect.
  assert.equal(
    canvasIngressOpen(EVENT_ID, false),
    true,
    "canvasIngressOpen keeps ingress open when eventId is non-null",
  );
});
