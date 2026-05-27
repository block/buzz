import assert from "node:assert/strict";
import test from "node:test";

// Imports the exact source the renderer (formatTimelineMessages.ts) and the
// post-edit cache-update (useEditMessageMutation) use. No inlined copy → no
// drift risk between test expectations and production behaviour.
import { applyEditTagOverlay } from "./applyEditTagOverlay.mjs";

const IMETA = (url) => ["imeta", `url ${url}`, "m image/png", "x x", "size 1"];

test("undefined editTags is a pass-through (returns original reference)", () => {
  const tags = [["h", "uuid"], IMETA("https://b/a.png")];
  assert.equal(applyEditTagOverlay(tags, undefined), tags);
});

test("does not mutate the original tag array", () => {
  const original = [["h", "uuid"], IMETA("https://b/a.png")];
  const snapshot = JSON.parse(JSON.stringify(original));
  const edit = [IMETA("https://b/c.png")];
  applyEditTagOverlay(original, edit);
  assert.deepEqual(original, snapshot);
});

test("edit replaces imeta A,B with edit's A,C; non-imeta from original survive", () => {
  const original = [
    ["h", "uuid"],
    ["p", "mention1"],
    IMETA("https://b/a.png"),
    IMETA("https://b/b.png"),
  ];
  const edit = [
    ["h", "uuid"],
    ["e", "originalEventId"],
    IMETA("https://b/a.png"),
    IMETA("https://b/c.png"),
  ];

  const out = applyEditTagOverlay(original, edit);

  // Non-imeta tags from the original survived (h, p mention).
  const nonImeta = out.filter((t) => t[0] !== "imeta");
  assert.deepEqual(nonImeta, [
    ["h", "uuid"],
    ["p", "mention1"],
  ]);

  // Imeta tags now match the edit's set (A,C — not B).
  const imetaUrls = out.filter((t) => t[0] === "imeta").map((t) => t[1]);
  assert.deepEqual(imetaUrls, ["url https://b/a.png", "url https://b/c.png"]);
});

test("edit with zero imeta tags strips all attachments; non-imeta original tags stay", () => {
  const original = [["h", "uuid"], IMETA("https://b/a.png")];
  const edit = [
    ["h", "uuid"],
    ["e", "x"],
  ];

  const out = applyEditTagOverlay(original, edit);
  assert.equal(out.filter((t) => t[0] === "imeta").length, 0);
  // h tag still present.
  assert.ok(out.some((t) => t[0] === "h"));
});

test("edit adds imeta to a previously text-only message; original mentions preserved", () => {
  const original = [
    ["h", "uuid"],
    ["p", "mention"],
  ];
  const edit = [["h", "uuid"], ["e", "x"], IMETA("https://b/a.png")];

  const out = applyEditTagOverlay(original, edit);
  const imeta = out.filter((t) => t[0] === "imeta");
  assert.equal(imeta.length, 1);
  assert.equal(imeta[0][1], "url https://b/a.png");
  // p mention still preserved from original.
  assert.ok(
    out.some((t) => t[0] === "p" && t[1] === "mention"),
    "non-imeta tags from original must be preserved",
  );
});

test("edit's non-imeta tags are dropped (only imeta wins)", () => {
  // The edit event itself carries `h` and `e` tags — the overlay must not
  // promote those into the merged set; only imeta tags from the edit win.
  const original = [
    ["h", "uuid-original"],
    ["p", "mention1"],
  ];
  const edit = [
    ["h", "uuid-from-edit-must-be-ignored"],
    ["e", "edit-target-event-id"],
    IMETA("https://b/a.png"),
  ];
  const out = applyEditTagOverlay(original, edit);
  // The original h survives, the edit's h is ignored.
  const hTags = out.filter((t) => t[0] === "h");
  assert.deepEqual(hTags, [["h", "uuid-original"]]);
  // No `e` tag from the edit leaked through.
  assert.equal(out.filter((t) => t[0] === "e").length, 0);
  // Original p mention still there.
  assert.ok(out.some((t) => t[0] === "p" && t[1] === "mention1"));
  // Imeta from the edit is present.
  assert.equal(out.filter((t) => t[0] === "imeta").length, 1);
});
