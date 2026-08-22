import assert from "node:assert/strict";
import test from "node:test";

import {
  clipboardPasteErrorMessage,
  firstClipboardFile,
  hasClipboardImageType,
  shouldReadNativeClipboardImage,
} from "./clipboardFile.ts";

const screenshot = { name: "screenshot.png", type: "image/png" };

test("firstClipboardFile returns a file exposed through clipboard items", () => {
  assert.equal(
    firstClipboardFile({
      files: [],
      items: [{ getAsFile: () => screenshot, kind: "file", type: "image/png" }],
    }),
    screenshot,
  );
});

test("firstClipboardFile falls back to clipboard files", () => {
  assert.equal(
    firstClipboardFile({
      files: [screenshot],
      items: [{ getAsFile: () => null, kind: "string", type: "text/plain" }],
    }),
    screenshot,
  );
});

test("hasClipboardImageType recognizes image MIME without a file", () => {
  assert.equal(
    hasClipboardImageType({ files: [], items: [], types: ["image/png"] }),
    true,
  );
});

test("native image fallback handles an empty WebKit paste payload", () => {
  assert.equal(
    shouldReadNativeClipboardImage({
      files: [],
      getData: () => "",
      items: [],
      types: [],
    }),
    true,
  );
});

test("native image fallback preserves ordinary text paste", () => {
  assert.equal(
    shouldReadNativeClipboardImage({
      files: [],
      getData: (type) => (type === "text/plain" ? "hello" : ""),
      items: [],
      types: ["text/plain"],
    }),
    false,
  );
});

test("native image fallback preserves non-image clipboard formats", () => {
  assert.equal(
    shouldReadNativeClipboardImage({
      files: [],
      getData: () => "",
      items: [],
      types: ["text/uri-list"],
    }),
    false,
  );
});

test("native image fallback preserves HTML-only paste", () => {
  assert.equal(
    shouldReadNativeClipboardImage({
      files: [],
      getData: (type) => (type === "text/html" ? "<strong>hello</strong>" : ""),
      items: [],
      types: ["text/html"],
    }),
    false,
  );
});

test("native image fallback prefers an advertised image in mixed clipboard data", () => {
  assert.equal(
    shouldReadNativeClipboardImage({
      files: [],
      getData: (type) => (type === "text/plain" ? "screenshot" : ""),
      items: [],
      types: ["text/plain", "image/png"],
    }),
    true,
  );
});

test("native image fallback recognizes an image item that WebKit cannot materialize", () => {
  assert.equal(
    shouldReadNativeClipboardImage({
      files: [],
      getData: () => "",
      items: [
        {
          getAsFile: () => null,
          kind: "file",
          type: "image/png",
        },
      ],
      types: ["Files"],
    }),
    true,
  );
});

test("clipboardPasteErrorMessage distinguishes empty clipboard from failures", () => {
  assert.equal(
    clipboardPasteErrorMessage("clipboard contains no image"),
    "Clipboard does not contain an image.",
  );
  assert.equal(
    clipboardPasteErrorMessage(new Error("IPC channel closed")),
    "Could not paste the clipboard image.",
  );
});
