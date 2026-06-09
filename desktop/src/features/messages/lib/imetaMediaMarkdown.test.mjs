import assert from "node:assert/strict";
import test from "node:test";

// Import the REAL implementations (the runner strips TS types via
// test-loader.mjs). Earlier this file inlined stale copies of these functions,
// which silently drifted from source — the inlined formatImetaMediaLine had no
// generic-file branch at all, so it could never catch a regression there.
// Importing the real module closes that blind spot.
import {
  buildImetaTags,
  buildOutgoingMessage,
  formatImetaMediaLine,
  imetaMediaFromTags,
  mergeOutgoingTags,
  splitOutgoingTags,
  stripImetaMediaLines,
} from "./imetaMediaMarkdown.ts";

test("strip: removes trailing image line whose URL is in imetaMedia", () => {
  const body = "Look at this\n![image](https://blossom/abc.png)";
  const stripped = stripImetaMediaLines(body, [
    { url: "https://blossom/abc.png", type: "image/png" },
  ]);
  assert.equal(stripped, "Look at this");
});

test("strip: removes trailing video line", () => {
  const body = "Demo:\n![video](https://blossom/clip.mp4)";
  const stripped = stripImetaMediaLines(body, [
    { url: "https://blossom/clip.mp4", type: "video/mp4" },
  ]);
  assert.equal(stripped, "Demo:");
});

test("strip: removes multiple trailing media lines in order", () => {
  const body = "two pics\n![image](https://b/a.png)\n![image](https://b/b.png)";
  const stripped = stripImetaMediaLines(body, [
    { url: "https://b/a.png", type: "image/png" },
    { url: "https://b/b.png", type: "image/png" },
  ]);
  assert.equal(stripped, "two pics");
});

test("strip: leaves body alone when no imeta entries", () => {
  const body = "hello\n![image](https://b/a.png)";
  assert.equal(stripImetaMediaLines(body, []), body);
});

test("strip: leaves media line whose URL isn't in imetaMedia", () => {
  const body = "hello\n![image](https://b/other.png)";
  const stripped = stripImetaMediaLines(body, [
    { url: "https://b/known.png", type: "image/png" },
  ]);
  assert.equal(stripped, body);
});

test("strip: stops at first non-media line (interleaved text preserved)", () => {
  const body =
    "before\n![image](https://b/a.png)\nmiddle\n![image](https://b/b.png)";
  const stripped = stripImetaMediaLines(body, [
    { url: "https://b/a.png", type: "image/png" },
    { url: "https://b/b.png", type: "image/png" },
  ]);
  assert.equal(stripped, "before\n![image](https://b/a.png)\nmiddle");
});

test("strip: tolerates blank lines between text and trailing media", () => {
  const body = "hi\n\n![image](https://b/a.png)";
  const stripped = stripImetaMediaLines(body, [
    { url: "https://b/a.png", type: "image/png" },
  ]);
  assert.equal(stripped, "hi");
});

// ── formatImetaMediaLine (send-path body markdown) ────────────────────

test("formatImetaMediaLine: image mime → ![image] line", () => {
  assert.equal(
    formatImetaMediaLine({ url: "https://b/a.png", type: "image/png" }),
    "\n![image](https://b/a.png)",
  );
});

test("buildImetaTags omits image filenames from imeta", () => {
  assert.deepEqual(
    buildImetaTags([
      {
        url: "https://b/a.png",
        type: "image/png",
        sha256: "abc",
        size: 10,
        uploaded: 1,
        filename: "Party Parrot.png",
      },
    ]),
    [["imeta", "url https://b/a.png", "m image/png", "x abc", "size 10"]],
  );
});

test("formatImetaMediaLine: video mime → ![video] line (regardless of URL suffix)", () => {
  assert.equal(
    formatImetaMediaLine({ url: "https://cdn/blob/xyz", type: "video/mp4" }),
    "\n![video](https://cdn/blob/xyz)",
  );
});

test("formatImetaMediaLine: generic mime → [filename](url) link", () => {
  assert.equal(
    formatImetaMediaLine({
      url: "https://b/blob",
      type: "application/pdf",
      filename: "report.pdf",
    }),
    "\n[report.pdf](https://b/blob)",
  );
});

test("formatImetaMediaLine: escapes markdown brackets/backslash in filename", () => {
  // `a].pdf` would otherwise close the link label early and break the FileCard.
  assert.equal(
    formatImetaMediaLine({
      url: "https://b/blob",
      type: "application/zip",
      filename: "a]b[c\\d.zip",
    }),
    "\n[a\\]b\\[c\\\\d.zip](https://b/blob)",
  );
});

test("strip: removes an escaped-bracket generic file line on edit", () => {
  // The escaped label must still be recognised by FILE_LINE_RE so the body is
  // cleaned in edit mode (regression guard for the FILE_LINE_RE escape support).
  const url = "https://b/blob";
  const body = `note${formatImetaMediaLine({ url, type: "application/pdf", filename: "a].pdf" })}`;
  const stripped = stripImetaMediaLines(body, [
    { url, type: "application/pdf" },
  ]);
  assert.equal(stripped, "note");
});

// ── imetaMediaFromTags (full BlobDescriptor projection) ───────────────

test("imetaMediaFromTags: empty / undefined", () => {
  assert.deepEqual(imetaMediaFromTags(undefined), []);
  assert.deepEqual(imetaMediaFromTags([]), []);
});

test("imetaMediaFromTags: full descriptor round-trip with all fields", () => {
  const tags = [
    [
      "imeta",
      "url https://b/photo.png",
      "m image/png",
      "x deadbeef",
      "size 12345",
      "dim 1920x1080",
      "blurhash LKO2:N%2Tw=^$f",
      "thumb https://b/photo-thumb.png",
      "image https://b/photo.png",
    ],
  ];
  const out = imetaMediaFromTags(tags);
  assert.deepEqual(out, [
    {
      url: "https://b/photo.png",
      type: "image/png",
      sha256: "deadbeef",
      size: 12345,
      uploaded: 0,
      dim: "1920x1080",
      blurhash: "LKO2:N%2Tw=^$f",
      thumb: "https://b/photo-thumb.png",
      image: "https://b/photo.png",
    },
  ]);
});

test("imetaMediaFromTags: video preserves duration", () => {
  const tags = [
    [
      "imeta",
      "url https://b/clip.mp4",
      "m video/mp4",
      "x cafef00d",
      "size 999000",
      "duration 12.5",
    ],
  ];
  const out = imetaMediaFromTags(tags);
  assert.equal(out.length, 1);
  assert.equal(out[0].duration, 12.5);
  assert.equal(out[0].type, "video/mp4");
});

test("imetaMediaFromTags: legacy entry without `m` falls back to image/jpeg", () => {
  const tags = [["imeta", "url https://b/legacy.jpg", "x abc", "size 100"]];
  const out = imetaMediaFromTags(tags);
  assert.equal(out.length, 1);
  assert.equal(out[0].type, "image/jpeg");
  assert.equal(out[0].sha256, "abc");
});

test("imetaMediaFromTags: skips entries without a url", () => {
  const tags = [["imeta", "m image/png", "x abc"]];
  assert.deepEqual(imetaMediaFromTags(tags), []);
});

test("imetaMediaFromTags: ignores non-imeta tags", () => {
  const tags = [
    ["e", "abc"],
    ["p", "def"],
    ["h", "uuid"],
  ];
  assert.deepEqual(imetaMediaFromTags(tags), []);
});

test("imetaMediaFromTags: preserves order across multiple entries", () => {
  const tags = [
    ["imeta", "url https://b/a.png", "m image/png", "x 1", "size 10"],
    ["imeta", "url https://b/b.png", "m image/png", "x 2", "size 20"],
    ["imeta", "url https://b/c.mp4", "m video/mp4", "x 3", "size 30"],
  ];
  const out = imetaMediaFromTags(tags);
  assert.deepEqual(
    out.map((d) => d.url),
    ["https://b/a.png", "https://b/b.png", "https://b/c.mp4"],
  );
});

// ── buildImetaTags (send + edit symmetry) ─────────────────────────────

test("buildImetaTags: round-trips through imetaMediaFromTags losslessly (full fields)", () => {
  const original = [
    {
      url: "https://b/photo.png",
      type: "image/png",
      sha256: "deadbeef",
      size: 12345,
      uploaded: 0,
      dim: "1920x1080",
      blurhash: "LKO2:N%2Tw=^$f",
      thumb: "https://b/photo-thumb.png",
      image: "https://b/photo.png",
    },
  ];
  const tags = buildImetaTags(original);
  const projected = imetaMediaFromTags(tags);
  assert.deepEqual(projected, original);
});

test("buildImetaTags: omits absent optional fields", () => {
  const tags = buildImetaTags([
    {
      url: "https://b/a.png",
      type: "image/png",
      sha256: "x",
      size: 1,
      uploaded: 0,
    },
  ]);
  assert.deepEqual(tags, [
    ["imeta", "url https://b/a.png", "m image/png", "x x", "size 1"],
  ]);
});

// ── Edit flow: open-edit → user modifies attachments → save ───────────

test("edit flow: imeta tags rebuilt from current pending after user removes one", () => {
  // Original event has two attachments.
  const originalTags = [
    ["imeta", "url https://b/a.png", "m image/png", "x 1", "size 10"],
    ["imeta", "url https://b/b.png", "m image/png", "x 2", "size 20"],
  ];

  // Composer projects them into pendingImeta on edit-load.
  const pending = imetaMediaFromTags(originalTags);
  assert.equal(pending.length, 2);

  // User removes the first one.
  const after = pending.filter((d) => d.url !== "https://b/a.png");

  // Composer builds the edit's mediaTags from the remaining pending list.
  const editMediaTags = buildImetaTags(after);
  assert.equal(editMediaTags.length, 1);
  assert.equal(editMediaTags[0][1], "url https://b/b.png");
});

// ── buildOutgoingMessage (shared body+tags builder for send + edit) ───

test("buildOutgoingMessage: empty pendingImeta returns body untouched and undefined mediaTags", () => {
  const out = buildOutgoingMessage("hello", []);
  assert.equal(out.content, "hello");
  assert.equal(out.mediaTags, undefined);
});

test("buildOutgoingMessage: appends media markdown line per attachment, in order", () => {
  const out = buildOutgoingMessage("hi", [
    {
      url: "https://b/a.png",
      type: "image/png",
      sha256: "x",
      size: 1,
      uploaded: 0,
    },
    {
      url: "https://b/v.mp4",
      type: "video/mp4",
      sha256: "y",
      size: 2,
      uploaded: 0,
    },
  ]);
  assert.equal(
    out.content,
    "hi\n![image](https://b/a.png)\n![video](https://b/v.mp4)",
  );
});

test("buildOutgoingMessage: mediaTags mirror buildImetaTags output for non-empty pending", () => {
  const pending = [
    {
      url: "https://b/a.png",
      type: "image/png",
      sha256: "abc",
      size: 99,
      uploaded: 0,
    },
  ];
  const out = buildOutgoingMessage("", pending);
  assert.deepEqual(out.mediaTags, buildImetaTags(pending));
});

// ── Sparse / legacy hygiene: omit empty x and zero size ───────────────

test("imetaMediaFromTags: entry without x leaves sha256 empty", () => {
  const tags = [["imeta", "url https://b/a.png", "m image/png", "size 1"]];
  const out = imetaMediaFromTags(tags);
  assert.equal(out.length, 1);
  assert.equal(out[0].sha256, "");
});

test("imetaMediaFromTags: entry without size leaves size 0", () => {
  const tags = [["imeta", "url https://b/a.png", "m image/png", "x deadbeef"]];
  const out = imetaMediaFromTags(tags);
  assert.equal(out.length, 1);
  assert.equal(out[0].size, 0);
});

test("buildImetaTags: omits x line when sha256 is empty", () => {
  const tags = buildImetaTags([
    {
      url: "https://b/a.png",
      type: "image/png",
      sha256: "",
      size: 1,
      uploaded: 0,
    },
  ]);
  assert.equal(tags.length, 1);
  // No element starts with "x " or "x\t" — no empty x line emitted.
  assert.ok(
    !tags[0].some((part) => /^x[\s\t]/.test(part)),
    `expected no x line, got ${JSON.stringify(tags[0])}`,
  );
});

test("buildImetaTags: omits size line when size is 0", () => {
  const tags = buildImetaTags([
    {
      url: "https://b/a.png",
      type: "image/png",
      sha256: "deadbeef",
      size: 0,
      uploaded: 0,
    },
  ]);
  assert.equal(tags.length, 1);
  assert.ok(
    !tags[0].some((part) => /^size[\s\t]/.test(part)),
    `expected no size line, got ${JSON.stringify(tags[0])}`,
  );
});

test("round-trip: sparse imeta from legacy tags rebuilds without empty x/size", () => {
  // Legacy / cross-client entry: only url + m. No x, no size.
  const legacyTags = [["imeta", "url https://b/legacy.png", "m image/png"]];
  const projected = imetaMediaFromTags(legacyTags);
  assert.equal(projected.length, 1);
  assert.equal(projected[0].sha256, "");
  assert.equal(projected[0].size, 0);

  const rebuilt = buildImetaTags(projected);
  assert.equal(rebuilt.length, 1);
  // Neither "x " nor "size 0" leaked into the rebuilt tag.
  assert.ok(
    !rebuilt[0].some((part) => /^x[\s\t]/.test(part)),
    `expected no x line, got ${JSON.stringify(rebuilt[0])}`,
  );
  assert.ok(
    !rebuilt[0].some((part) => /^size[\s\t]/.test(part)),
    `expected no size line, got ${JSON.stringify(rebuilt[0])}`,
  );
  // url and m survived.
  assert.deepEqual(rebuilt[0], [
    "imeta",
    "url https://b/legacy.png",
    "m image/png",
  ]);
});

const IMETA = ["imeta", "url https://blossom/abc.png", "m image/png"];
const EMOJI_A = ["emoji", "shipit", "https://relay/s.png"];
const EMOJI_B = ["emoji", "party", "https://relay/p.gif"];

test("splitOutgoingTags: undefined input yields two empty arrays", () => {
  assert.deepEqual(splitOutgoingTags(undefined), {
    mediaTags: [],
    emojiTags: [],
  });
});

test("splitOutgoingTags: separates emoji tags from imeta tags", () => {
  const { mediaTags, emojiTags } = splitOutgoingTags([IMETA, EMOJI_A, EMOJI_B]);
  assert.deepEqual(mediaTags, [IMETA]);
  assert.deepEqual(emojiTags, [EMOJI_A, EMOJI_B]);
});

test("splitOutgoingTags: emoji-only set leaves mediaTags empty", () => {
  const { mediaTags, emojiTags } = splitOutgoingTags([EMOJI_A]);
  assert.deepEqual(mediaTags, []);
  assert.deepEqual(emojiTags, [EMOJI_A]);
});

test("splitOutgoingTags: unknown prefixes stay with mediaTags (injection defense)", () => {
  // A forged ["p", ...] must NOT be misrouted to the emoji channel; it stays on
  // mediaTags where the server-side imeta guard rejects it.
  const forged = ["p", "deadbeef"];
  const { mediaTags, emojiTags } = splitOutgoingTags([forged, EMOJI_A]);
  assert.deepEqual(mediaTags, [forged]);
  assert.deepEqual(emojiTags, [EMOJI_A]);
});

test("splitOutgoingTags is the inverse of mergeOutgoingTags", () => {
  const merged = mergeOutgoingTags([IMETA], [EMOJI_A, EMOJI_B]);
  const { mediaTags, emojiTags } = splitOutgoingTags(merged);
  assert.deepEqual(mediaTags, [IMETA]);
  assert.deepEqual(emojiTags, [EMOJI_A, EMOJI_B]);
});
