import assert from "node:assert/strict";
import { test } from "node:test";

import { isRelayDownloadable, isVideoMedia } from "./mediaEntry.ts";

const RELAY = "https://relay.example.com";
const relayUrl = (name) => `${RELAY}/media/${name}`;

// ── isVideoMedia: MIME-first classification ──────────────────────────────

test("isVideoMedia: video/* MIME classifies as video regardless of extension", () => {
  assert.equal(isVideoMedia(relayUrl("abc"), "video/mp4"), true);
  assert.equal(isVideoMedia(relayUrl("abc.jpg"), "video/webm"), true);
});

test("isVideoMedia: MIME wins for an extensionless relay URL", () => {
  // The relay path is a content hash with no extension; MIME is the only signal.
  assert.equal(isVideoMedia(relayUrl("deadbeef"), "video/quicktime"), true);
});

test("isVideoMedia: uppercase MIME still matches", () => {
  assert.equal(isVideoMedia(relayUrl("abc"), "VIDEO/MP4"), true);
});

test("isVideoMedia: image MIME is not a video", () => {
  assert.equal(isVideoMedia(relayUrl("abc.mp4"), "image/png"), false);
});

// ── isVideoMedia: legacy extension fallback (no MIME) ─────────────────────

test("isVideoMedia: legacy mp4/webm/mov extensions classify as video", () => {
  assert.equal(isVideoMedia(relayUrl("abc.mp4")), true);
  assert.equal(isVideoMedia(relayUrl("abc.webm")), true);
  assert.equal(isVideoMedia(relayUrl("abc.mov")), true);
});

test("isVideoMedia: uppercase extension classifies as video", () => {
  assert.equal(isVideoMedia(relayUrl("abc.MP4")), true);
  assert.equal(isVideoMedia(relayUrl("abc.WebM")), true);
});

test("isVideoMedia: extension with a query string still classifies", () => {
  assert.equal(isVideoMedia(relayUrl("abc.mp4?v=2")), true);
  assert.equal(isVideoMedia(relayUrl("abc.webm#t=10")), true);
});

test("isVideoMedia: image extensions are not videos", () => {
  assert.equal(isVideoMedia(relayUrl("abc.jpg")), false);
  assert.equal(isVideoMedia(relayUrl("abc.png")), false);
});

test("isVideoMedia: an extension substring is not enough", () => {
  // "notmp4" must not match "mp4" — only the real trailing extension counts.
  assert.equal(isVideoMedia(relayUrl("clip.notmp4")), false);
});

test("isVideoMedia: malformed / extensionless URL without MIME is not a video", () => {
  assert.equal(isVideoMedia("not a url"), false);
  assert.equal(isVideoMedia(relayUrl("deadbeef")), false);
});

// ── isRelayDownloadable: eligibility, independent of render kind ──────────

test("isRelayDownloadable: relay /media/ URL on the resolved origin is eligible", () => {
  assert.equal(isRelayDownloadable(relayUrl("abc.mp4"), RELAY), true);
});

test("isRelayDownloadable: off-relay URL is not eligible", () => {
  assert.equal(
    isRelayDownloadable("https://evil.example.com/media/abc.mp4", RELAY),
    false,
  );
});

test("isRelayDownloadable: non-/media/ path on the relay is not eligible", () => {
  assert.equal(isRelayDownloadable(`${RELAY}/other/abc.mp4`, RELAY), false);
});

test("isRelayDownloadable: malformed URL is not eligible", () => {
  assert.equal(isRelayDownloadable("not a url", RELAY), false);
});

test("isRelayDownloadable: unresolved origin fails closed (no Download offered)", () => {
  // Before the relay origin resolves we can't distinguish a relay /media/ URL
  // from an off-relay one, so eligibility fails closed — offering Download for
  // an off-relay URL would only error, and the Rust gate rejects it anyway.
  // Callers read the origin from a reactive source so eligibility recomputes
  // once it resolves (see the transition cases below).
  assert.equal(isRelayDownloadable(relayUrl("abc.mp4")), false);
  assert.equal(isRelayDownloadable(relayUrl("abc.mp4"), undefined), false);
  assert.equal(
    isRelayDownloadable("https://evil.example.com/media/abc.mp4"),
    false,
  );
});

test("isRelayDownloadable: unresolved → relay origin makes a relay URL eligible", () => {
  const src = relayUrl("abc.mp4");
  // First render: origin unresolved → Download hidden.
  assert.equal(isRelayDownloadable(src, undefined), false);
  // Origin resolves to the relay → the same URL becomes eligible.
  assert.equal(isRelayDownloadable(src, RELAY), true);
});

test("isRelayDownloadable: unresolved → off-relay origin keeps an external URL hidden", () => {
  const src = "https://evil.example.com/media/abc.mp4";
  // First render: origin unresolved → hidden.
  assert.equal(isRelayDownloadable(src, undefined), false);
  // Origin resolves to a different (relay) origin → still hidden.
  assert.equal(isRelayDownloadable(src, RELAY), false);
});
