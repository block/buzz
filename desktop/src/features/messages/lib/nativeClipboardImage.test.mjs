import assert from "node:assert/strict";
import test from "node:test";

import {
  clipboardImageFile,
  clipboardImageFilename,
  isPasteShortcut,
  shouldReadNativeClipboardImage,
} from "./nativeClipboardImage.ts";

function clipboard({ types = [], items = [], files = [] } = {}) {
  return { files, items, types };
}

test("an empty DataTransfer from a keyboard paste falls back", () => {
  assert.equal(shouldReadNativeClipboardImage(clipboard(), true), true);
});

test("the same empty DataTransfer from a middle-click paste does not", () => {
  // WebKitGTK reports an identical empty DataTransfer for a middle-click
  // paste of PRIMARY, while arboard would read an unrelated CLIPBOARD image.
  assert.equal(shouldReadNativeClipboardImage(clipboard(), false), false);
});

test("a text paste is never taken over", () => {
  assert.equal(
    shouldReadNativeClipboardImage(clipboard({ types: ["text/plain"] }), true),
    false,
  );
  assert.equal(
    shouldReadNativeClipboardImage(
      clipboard({ types: ["text/html", "text/plain"] }),
      true,
    ),
    false,
  );
});

test("an advertised image alongside text is left to the text handlers", () => {
  assert.equal(
    shouldReadNativeClipboardImage(
      clipboard({ types: ["text/html", "image/png"] }),
      true,
    ),
    false,
  );
});

test("a file paste is never taken over", () => {
  assert.equal(
    shouldReadNativeClipboardImage(
      clipboard({ files: [{}], items: [{ kind: "file" }] }),
      true,
    ),
    false,
  );
});

test("a missing DataTransfer does not trigger a native read", () => {
  assert.equal(shouldReadNativeClipboardImage(null, true), false);
  assert.equal(shouldReadNativeClipboardImage(undefined, true), false);
});

test("only Ctrl/Cmd+V counts as a paste shortcut", () => {
  assert.equal(isPasteShortcut({ ctrlKey: true, key: "v" }), true);
  assert.equal(isPasteShortcut({ metaKey: true, key: "V" }), true);
  assert.equal(isPasteShortcut({ ctrlKey: true, key: "c" }), false);
  assert.equal(isPasteShortcut({ key: "v" }), false);
});

test("the generated filename is a PNG with no characters a path rejects", () => {
  const name = clipboardImageFilename(new Date("2026-09-22T14:05:09.123Z"));
  assert.equal(name, "pasted-image-2026-09-22T14-05-09Z.png");
  assert.match(name, /^[A-Za-z0-9._-]+\.png$/);
});

test("clipboard bytes become a PNG File the upload path accepts", async () => {
  const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47]).buffer;
  const file = clipboardImageFile(png);

  assert.equal(file.type, "image/png");
  assert.match(file.name, /^pasted-image-.*\.png$/);
  assert.equal(file.size, 4);
  assert.deepEqual(
    new Uint8Array(await file.arrayBuffer()),
    new Uint8Array([0x89, 0x50, 0x4e, 0x47]),
  );
});
