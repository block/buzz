import assert from "node:assert/strict";
import test from "node:test";

import {
  buildLinkPreviewSnapshotTag,
  parseLinkPreviewSnapshots,
} from "./linkPreviewSnapshot.ts";

const HASH = "a".repeat(64);
const ORIGIN = "https://relay.example";
const CONTENT = "Read https://linear.app/acme/issue/ABC-123/example";
const URL = "https://linear.app/acme/issue/ABC-123/example";
const valid = [
  "link-preview",
  "snapshot",
  "1",
  URL,
  "Example",
  "Linear",
  "Description",
  `${ORIGIN}/media/${HASH}.png`,
  HASH,
  "",
  "",
];

test("authored snapshots render only exact local media tied to message content", () => {
  assert.equal(parseLinkPreviewSnapshots([valid], CONTENT, ORIGIN).length, 1);
  assert.equal(
    parseLinkPreviewSnapshots([valid], "no matching link", ORIGIN).length,
    0,
  );
  assert.equal(parseLinkPreviewSnapshots([valid], CONTENT, null).length, 0);
});

test("authored snapshots reject remote, malformed, credentialed, and hash-mismatched media", () => {
  for (const image of [
    `https://evil.example/media/${HASH}.png`,
    `${ORIGIN}/media/${HASH}.png?token=leak`,
    `${ORIGIN}/media/${HASH}.png#fragment`,
    `https://user@relay.example/media/${HASH}.png`,
    `${ORIGIN}/media/${HASH}.svg`,
    `${ORIGIN}/media/${HASH}.png/extra`,
  ]) {
    const tag = [...valid];
    tag[7] = image;
    assert.equal(
      parseLinkPreviewSnapshots([tag], CONTENT, ORIGIN).length,
      0,
      image,
    );
  }
  const mismatch = [...valid];
  mismatch[8] = "b".repeat(64);
  assert.equal(
    parseLinkPreviewSnapshots([mismatch], CONTENT, ORIGIN).length,
    0,
  );
});

test("snapshot tags sanitize control characters and enforce UTF-8 byte limits", () => {
  const tag = buildLinkPreviewSnapshotTag({
    canonicalUrl: URL,
    title: `Title\n${"😀".repeat(100)}`,
    siteName: `Linear\u0000${"é".repeat(100)}`,
    description: `First\nSecond\t${"😀".repeat(300)}`,
    imageUrl: "",
    imageSha256: "",
    faviconUrl: "",
    faviconSha256: "",
  });

  assert.ok(tag);
  assert.equal(tag[6].startsWith("First\nSecond"), true);
  assert.equal(tag[6].includes("\t"), false);
  for (const value of tag.slice(4, 6)) {
    assert.ok(
      Array.from(value).every((char) => {
        const code = char.charCodeAt(0);
        return code > 0x1f && code !== 0x7f;
      }),
    );
  }
  assert.ok(Buffer.byteLength(tag[4], "utf8") <= 300);
  assert.ok(Buffer.byteLength(tag[5], "utf8") <= 100);
  assert.ok(Buffer.byteLength(tag[6], "utf8") <= 1000);
});

test("snapshot tags omit canonical URLs rejected by native validation", () => {
  for (const canonicalUrl of [
    `${URL}#`,
    `${URL}#details`,
    `http://linear.app/acme/issue/ABC-123/example`,
    `https://user@linear.app/acme/issue/ABC-123/example`,
  ]) {
    assert.equal(
      buildLinkPreviewSnapshotTag({
        canonicalUrl,
        title: "Example",
        siteName: "Linear",
        description: "Description",
        imageUrl: "",
        imageSha256: "",
        faviconUrl: "",
        faviconSha256: "",
      }),
      null,
      canonicalUrl,
    );
  }
});

test("messages without authored snapshots never create recipient previews", () => {
  assert.deepEqual(parseLinkPreviewSnapshots([], CONTENT, ORIGIN), []);
  assert.deepEqual(parseLinkPreviewSnapshots(undefined, CONTENT, ORIGIN), []);
});
