/**
 * Regression tests for the sticker send path's three review findings.
 *
 * These import the ACTUAL exported helpers from stickerComposerSend.ts rather
 * than restating their logic, so reverting the behaviour at the call site or
 * inside the helper breaks these tests immediately.
 *
 * Properties under test, one per review finding on PR #2968:
 *   (a) a sticker send never carries typed text — renderers take the sticker
 *       branch and drop the markdown body, so typed text would vanish silently
 *   (b) a sticker send never carries pending attachments, for the same reason
 *   (c) the pre-send draft is restored only when the composer is still empty,
 *       so edits made while the send is in flight are not clobbered
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  captureStickerDraft,
  shouldRestoreStickerDraft,
  stickerSendOverrides,
} from "./stickerComposerSend.ts";

const SELECTION = {
  pack: {
    author: "a".repeat(64),
    identifier: "party-pack",
    eventId: "b".repeat(64),
  },
  sticker: { shortcode: "tada", url: "https://relay.example/x.webp" },
};

const IMETA = [{ url: "https://relay.example/photo.png", mime: "image/png" }];

test("sticker send replaces typed text with the shortcode fallback", () => {
  const overrides = stickerSendOverrides(
    SELECTION,
    [],
    new Set(),
    "look at this",
  );
  assert.equal(overrides.trimmed, ":tada:");
  assert.notEqual(overrides.trimmed, "look at this");
});

test("sticker send withholds pending attachments and their spoiler flags", () => {
  const overrides = stickerSendOverrides(
    SELECTION,
    IMETA,
    new Set(["https://relay.example/photo.png"]),
    "",
  );
  assert.deepEqual(overrides.pendingImeta, []);
  assert.equal(overrides.spoileredAttachmentUrls.size, 0);
});

test("sticker send emits exactly one sticker reference tag", () => {
  const overrides = stickerSendOverrides(SELECTION, [], new Set(), "");
  assert.equal(overrides.stickerTags.length, 1);
});

test("a normal send passes draft state through untouched", () => {
  const spoilered = new Set(["https://relay.example/photo.png"]);
  const overrides = stickerSendOverrides(
    undefined,
    IMETA,
    spoilered,
    "hello world",
  );
  assert.equal(overrides.trimmed, "hello world");
  assert.equal(overrides.pendingImeta, IMETA);
  assert.equal(overrides.spoileredAttachmentUrls, spoilered);
  assert.deepEqual(overrides.stickerTags, []);
});

test("captureStickerDraft returns null for a non-sticker send", () => {
  assert.equal(captureStickerDraft(undefined, "draft", IMETA, new Set()), null);
});

test("captureStickerDraft copies imeta and spoiler sets defensively", () => {
  const spoilered = new Set(["https://relay.example/photo.png"]);
  const snapshot = captureStickerDraft(SELECTION, "draft", IMETA, spoilered);
  assert.notEqual(snapshot.imeta, IMETA);
  assert.notEqual(snapshot.spoileredAttachmentUrls, spoilered);
  assert.deepEqual(snapshot.imeta, IMETA);
  assert.equal(snapshot.content, "draft");
});

test("draft is restored when the composer is still empty after the send", () => {
  const snapshot = captureStickerDraft(SELECTION, "draft", [], new Set());
  assert.equal(shouldRestoreStickerDraft(snapshot, ""), true);
  assert.equal(shouldRestoreStickerDraft(snapshot, "   "), true);
});

test("draft is NOT restored over text typed while the send was in flight", () => {
  const snapshot = captureStickerDraft(SELECTION, "draft", [], new Set());
  assert.equal(shouldRestoreStickerDraft(snapshot, "typed meanwhile"), false);
});

test("nothing is restored for a non-sticker send", () => {
  assert.equal(shouldRestoreStickerDraft(null, ""), false);
});
