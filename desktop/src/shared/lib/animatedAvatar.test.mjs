import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAnimatedAvatarUrl,
  getAvatarSnapshotUrl,
  parseAnimatedAvatarUrl,
} from "./animatedAvatar.ts";

const POSTER = "https://relay.example.com/media/aa.png";
const GIF = "https://relay.example.com/media/bb.gif";

// ── round trip ───────────────────────────────────────────────────────────────

test("build + parse round-trips poster and gif URLs", () => {
  const url = buildAnimatedAvatarUrl(POSTER, GIF);
  const parsed = parseAnimatedAvatarUrl(url);
  assert.deepEqual(parsed, { posterUrl: POSTER, animationUrl: GIF });
});

test("build encodes the gif URL so the fragment has no reserved chars", () => {
  const gif = "https://relay.example.com/media/bb.gif?x=1&y=2#frag";
  const url = buildAnimatedAvatarUrl(POSTER, gif);
  const fragment = url.slice(url.indexOf("#buzz-anim=") + "#buzz-anim=".length);
  assert.ok(!fragment.includes("#"), "fragment should not contain raw #");
  assert.ok(!fragment.includes("&"), "fragment should not contain raw &");
  assert.equal(parseAnimatedAvatarUrl(url)?.animationUrl, gif);
});

// ── parse rejects non-animated URLs ──────────────────────────────────────────

test("parse returns null for plain image URLs", () => {
  assert.equal(parseAnimatedAvatarUrl(POSTER), null);
});

test("parse returns null for null/undefined/empty", () => {
  assert.equal(parseAnimatedAvatarUrl(null), null);
  assert.equal(parseAnimatedAvatarUrl(undefined), null);
  assert.equal(parseAnimatedAvatarUrl(""), null);
});

test("parse returns null for emoji data-url avatars", () => {
  assert.equal(
    parseAnimatedAvatarUrl("data:image/svg+xml,%3Csvg%3E%3C/svg%3E"),
    null,
  );
});

test("parse returns null when fragment marker has no poster prefix", () => {
  assert.equal(parseAnimatedAvatarUrl(`#buzz-anim=${GIF}`), null);
});

test("parse returns null when gif part is empty", () => {
  assert.equal(parseAnimatedAvatarUrl(`${POSTER}#buzz-anim=`), null);
});

test("parse returns null when gif part is not an http(s) URL", () => {
  assert.equal(
    parseAnimatedAvatarUrl(`${POSTER}#buzz-anim=javascript%3Aalert(1)`),
    null,
  );
});

test("parse returns null when poster part is not an http(s) URL", () => {
  assert.equal(
    parseAnimatedAvatarUrl(
      `data:image/png;base64,xx#buzz-anim=${encodeURIComponent(GIF)}`,
    ),
    null,
  );
});

test("parse returns null on malformed percent-encoding", () => {
  assert.equal(parseAnimatedAvatarUrl(`${POSTER}#buzz-anim=%E0%A4%A`), null);
});

// -- snapshot URL -------------------------------------------------------------

test("getAvatarSnapshotUrl strips animated avatars to their poster URL", () => {
  assert.equal(
    getAvatarSnapshotUrl(buildAnimatedAvatarUrl(POSTER, GIF)),
    POSTER,
  );
});

test("getAvatarSnapshotUrl preserves plain avatar URLs", () => {
  assert.equal(getAvatarSnapshotUrl(POSTER), POSTER);
});

test("getAvatarSnapshotUrl returns null for nullish avatar URLs", () => {
  assert.equal(getAvatarSnapshotUrl(null), null);
  assert.equal(getAvatarSnapshotUrl(undefined), null);
});
