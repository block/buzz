import assert from "node:assert/strict";
import test from "node:test";

// ── Inlined pure functions from imetaMediaMarkdown.ts ─────────────────
// Inlined to avoid importing from .ts files (no TS loader in node:test).
// Same pattern as markdown.test.mjs / useMediaUpload.test.mjs.

const MEDIA_LINE_RE = /^!\[(?:image|video)\]\(([^)\s]+)\)\s*$/;

function stripImetaMediaLines(body, imetaMedia) {
  if (imetaMedia.length === 0) return body;
  const urls = new Set(imetaMedia.map((m) => m.url));
  const lines = body.split("\n");
  let end = lines.length;
  while (end > 0) {
    const line = lines[end - 1];
    if (line.trim() === "") {
      end -= 1;
      continue;
    }
    const match = line.match(MEDIA_LINE_RE);
    if (match && urls.has(match[1])) {
      end -= 1;
      continue;
    }
    break;
  }
  return lines.slice(0, end).join("\n").replace(/\s+$/, "");
}

function formatImetaMediaLine({ url, type }) {
  const isVideo = type.startsWith("video/");
  return isVideo ? `\n![video](${url})` : `\n![image](${url})`;
}

function buildImetaTags(imetaMedia) {
  return imetaMedia.map((d) => [
    "imeta",
    `url ${d.url}`,
    `m ${d.type}`,
    ...(d.sha256 ? [`x ${d.sha256}`] : []),
    ...(typeof d.size === "number" && d.size > 0 ? [`size ${d.size}`] : []),
    ...(d.dim ? [`dim ${d.dim}`] : []),
    ...(d.blurhash ? [`blurhash ${d.blurhash}`] : []),
    ...(d.thumb ? [`thumb ${d.thumb}`] : []),
    ...(d.duration != null ? [`duration ${d.duration}`] : []),
    ...(d.image ? [`image ${d.image}`] : []),
  ]);
}

function buildOutgoingMessage(body, pendingImeta) {
  let content = body;
  for (const d of pendingImeta) content += formatImetaMediaLine(d);
  const mediaTags =
    pendingImeta.length > 0 ? buildImetaTags(pendingImeta) : undefined;
  return { content, mediaTags };
}

// Mirror of `parseImetaTags` + `imetaMediaFromTags` so the projection's
// type/x/size/dim/blurhash/thumb/duration/image fields can be tested without
// a TS loader.
function parseImetaTagsInline(tags) {
  const map = new Map();
  for (const tag of tags) {
    if (tag[0] !== "imeta") continue;
    const entry = {};
    for (const part of tag.slice(1)) {
      const i = part.indexOf(" ");
      if (i === -1) continue;
      const key = part.slice(0, i);
      const val = part.slice(i + 1);
      if (key === "url") entry.url = val;
      else if (key === "m") entry.m = val;
      else if (key === "x") entry.x = val;
      else if (key === "size") entry.size = parseInt(val, 10);
      else if (key === "dim") entry.dim = val;
      else if (key === "blurhash") entry.blurhash = val;
      else if (key === "thumb") entry.thumb = val;
      else if (key === "duration") entry.duration = parseFloat(val);
      else if (key === "image") entry.image = val;
    }
    if (entry.url) map.set(entry.url, entry);
  }
  return map;
}

function imetaMediaFromTags(tags) {
  if (!tags || tags.length === 0) return [];
  const entries = parseImetaTagsInline(tags);
  const out = [];
  for (const e of entries.values()) {
    if (!e.url) continue;
    out.push({
      url: e.url,
      type: e.m ?? "image/jpeg",
      sha256: e.x ?? "",
      size: e.size ?? 0,
      uploaded: 0,
      ...(e.dim ? { dim: e.dim } : {}),
      ...(e.blurhash ? { blurhash: e.blurhash } : {}),
      ...(e.thumb ? { thumb: e.thumb } : {}),
      ...(e.duration != null ? { duration: e.duration } : {}),
      ...(e.image ? { image: e.image } : {}),
    });
  }
  return out;
}

// ── stripImetaMediaLines ──────────────────────────────────────────────

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

test("formatImetaMediaLine: video mime → ![video] line (regardless of URL suffix)", () => {
  assert.equal(
    formatImetaMediaLine({ url: "https://cdn/blob/xyz", type: "video/mp4" }),
    "\n![video](https://cdn/blob/xyz)",
  );
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
