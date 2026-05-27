import assert from "node:assert/strict";
import test from "node:test";

// Imports the exact source the renderer (formatTimelineMessages.ts) and the
// post-edit cache-update (useEditMessageMutation) use. No inlined copy → no
// drift risk between test expectations and production behaviour.
import { applyEditOverlay, applyEditTagOverlay } from "./applyEditOverlay.mjs";

const IMETA = (url) => ["imeta", `url ${url}`, "m image/png", "x x", "size 1"];

test("overlay: no edit returns original body and tags untouched", () => {
  const original = {
    content: "hello",
    tags: [["h", "uuid"], ["p", "abc"], IMETA("https://b/a.png")],
  };
  const out = applyEditOverlay(original, undefined);
  assert.equal(out.body, "hello");
  assert.deepEqual(out.tags, original.tags);
});

test("overlay: edit replaces imeta tags A,B with edit's A,C; non-imeta preserved", () => {
  const original = {
    content: "hi",
    tags: [
      ["h", "uuid"],
      ["p", "mention1"],
      IMETA("https://b/a.png"),
      IMETA("https://b/b.png"),
    ],
  };
  const edit = {
    content: "hi (edited)",
    tags: [
      ["h", "uuid"],
      ["e", "originalEventId"],
      IMETA("https://b/a.png"),
      IMETA("https://b/c.png"),
    ],
  };

  const out = applyEditOverlay(original, edit);

  assert.equal(out.body, "hi (edited)");

  // Non-imeta tags from the original survived (h, p mention).
  const nonImeta = out.tags.filter((t) => t[0] !== "imeta");
  assert.deepEqual(nonImeta, [
    ["h", "uuid"],
    ["p", "mention1"],
  ]);

  // Imeta tags now match the edit's set (A,C — not B).
  const imetaUrls = out.tags.filter((t) => t[0] === "imeta").map((t) => t[1]);
  assert.deepEqual(imetaUrls, ["url https://b/a.png", "url https://b/c.png"]);
});

test("overlay: edit with zero imeta tags strips all attachments", () => {
  const original = {
    content: "with media",
    tags: [["h", "uuid"], IMETA("https://b/a.png")],
  };
  const edit = {
    content: "media removed",
    tags: [
      ["h", "uuid"],
      ["e", "x"],
    ],
  };

  const out = applyEditOverlay(original, edit);
  assert.equal(out.body, "media removed");
  assert.equal(out.tags.filter((t) => t[0] === "imeta").length, 0);
  // h tag still present.
  assert.ok(out.tags.some((t) => t[0] === "h"));
});

test("overlay: edit adding imeta to a previously text-only message", () => {
  const original = {
    content: "just text",
    tags: [
      ["h", "uuid"],
      ["p", "mention"],
    ],
  };
  const edit = {
    content: "now with media",
    tags: [["h", "uuid"], ["e", "x"], IMETA("https://b/a.png")],
  };

  const out = applyEditOverlay(original, edit);
  const imeta = out.tags.filter((t) => t[0] === "imeta");
  assert.equal(imeta.length, 1);
  assert.equal(imeta[0][1], "url https://b/a.png");
  // p mention still preserved from original.
  assert.ok(
    out.tags.some((t) => t[0] === "p" && t[1] === "mention"),
    "non-imeta tags from original must be preserved",
  );
});

test("applyEditTagOverlay: undefined editTags is a pass-through", () => {
  const tags = [["h", "uuid"], IMETA("https://b/a.png")];
  assert.equal(applyEditTagOverlay(tags, undefined), tags);
});

test("applyEditTagOverlay: edit's non-imeta tags are dropped (only imeta wins)", () => {
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
