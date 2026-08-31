import assert from "node:assert/strict";
import test from "node:test";

import {
  basenameFromPath,
  extractDroppedFilePayload,
  extractPathsFromText,
  fileUriOrAbsolutePath,
  isOsFileDrag,
  looksLikeFileName,
} from "./droppedFiles.ts";

test("fileUriOrAbsolutePath accepts unix absolute paths", () => {
  assert.equal(
    fileUriOrAbsolutePath("/home/me/Pictures/cat.png"),
    "/home/me/Pictures/cat.png",
  );
});

test("fileUriOrAbsolutePath accepts file:// URIs", () => {
  assert.equal(
    fileUriOrAbsolutePath("file:///home/me/Pictures/cat.png"),
    "/home/me/Pictures/cat.png",
  );
});

test("fileUriOrAbsolutePath decodes percent-encoded names", () => {
  assert.equal(
    fileUriOrAbsolutePath("file:///home/me/My%20Photos/cat%20hat.png"),
    "/home/me/My Photos/cat hat.png",
  );
});

test("fileUriOrAbsolutePath strips the extra slash on Windows file URIs", () => {
  assert.equal(
    fileUriOrAbsolutePath("file:///C:/Users/me/Desktop/shot.jpg"),
    "C:/Users/me/Desktop/shot.jpg",
  );
});

test("fileUriOrAbsolutePath accepts Windows drive and UNC paths", () => {
  assert.equal(
    fileUriOrAbsolutePath(String.raw`C:\Users\me\a.png`),
    String.raw`C:\Users\me\a.png`,
  );
  assert.equal(
    fileUriOrAbsolutePath(String.raw`\\nas\share\photo.jpg`),
    String.raw`\\nas\share\photo.jpg`,
  );
});

test("fileUriOrAbsolutePath rejects http(s) and relative paths", () => {
  assert.equal(fileUriOrAbsolutePath("https://example.com/a.png"), null);
  assert.equal(fileUriOrAbsolutePath("http://example.com/a.png"), null);
  assert.equal(fileUriOrAbsolutePath("photos/cat.png"), null);
  assert.equal(fileUriOrAbsolutePath("file:not-a-url"), null);
  assert.equal(fileUriOrAbsolutePath(""), null);
});

test("extractPathsFromText skips uri-list comments and blanks", () => {
  const text = [
    "# comment",
    "file:///tmp/a.png",
    "",
    "/tmp/b.jpg",
    "https://example.com/c.png",
  ].join("\n");
  assert.deepEqual(extractPathsFromText(text), ["/tmp/a.png", "/tmp/b.jpg"]);
});

test("extractPathsFromText deduplicates", () => {
  assert.deepEqual(extractPathsFromText("/tmp/a.png\n/tmp/a.png\n"), [
    "/tmp/a.png",
  ]);
});

test("isOsFileDrag is true for Files and uri-list, not plain text", () => {
  assert.equal(isOsFileDrag({ types: ["Files"] }), true);
  assert.equal(isOsFileDrag({ types: ["text/uri-list", "text/plain"] }), true);
  assert.equal(isOsFileDrag({ types: ["text/plain"] }), false);
  assert.equal(isOsFileDrag({ types: ["text/html"] }), false);
  assert.equal(isOsFileDrag(null), false);
});

test("extractDroppedFilePayload prefers usable File objects over path text", () => {
  const file = { name: "a.png", size: 12 };
  const data = {
    files: [file],
    types: ["Files", "text/uri-list"],
    getData: () => "file:///tmp/ignored.png",
  };
  assert.deepEqual(extractDroppedFilePayload(data), {
    files: [file],
    paths: [],
  });
});

test("extractDroppedFilePayload ignores dummy File objects and reads paths", () => {
  const dummy = { name: "", size: 0 };
  const data = {
    files: [dummy],
    types: ["Files", "text/uri-list"],
    getData: (type) => (type === "text/uri-list" ? "file:///tmp/a.png" : ""),
  };
  assert.deepEqual(extractDroppedFilePayload(data), {
    files: [],
    paths: ["/tmp/a.png"],
  });
});

test("extractDroppedFilePayload reads GNOME copied-files lists", () => {
  const data = {
    files: [],
    types: ["x-special/gnome-copied-files"],
    getData: (type) =>
      type === "x-special/gnome-copied-files"
        ? "copy\nfile:///tmp/a.png"
        : "",
  };
  assert.deepEqual(extractDroppedFilePayload(data), {
    files: [],
    paths: ["/tmp/a.png"],
  });
});

test("extractDroppedFilePayload recovers paths when files is empty", () => {
  const data = {
    files: [],
    types: ["text/uri-list", "text/plain"],
    getData: (type) =>
      type === "text/uri-list"
        ? "file:///tmp/a.png\nfile:///tmp/b.jpg"
        : "/tmp/a.png",
  };
  assert.deepEqual(extractDroppedFilePayload(data), {
    files: [],
    paths: ["/tmp/a.png", "/tmp/b.jpg"],
  });
});

test("extractDroppedFilePayload is empty for ordinary text drags", () => {
  const data = {
    files: [],
    types: ["text/plain"],
    getData: () => "hello from another app",
  };
  assert.deepEqual(extractDroppedFilePayload(data), { files: [], paths: [] });
});

test("extractDroppedFilePayload ignores absolute paths without a file extension", () => {
  const data = {
    files: [],
    types: ["text/plain"],
    getData: () => "/usr/bin/env python\n/etc/passwd",
  };
  assert.deepEqual(extractDroppedFilePayload(data), { files: [], paths: [] });
});

test("looksLikeFileName requires a basename with an extension", () => {
  assert.equal(looksLikeFileName("/tmp/photo.png"), true);
  assert.equal(looksLikeFileName("/etc/passwd"), false);
  assert.equal(looksLikeFileName("/usr/bin/env python"), false);
});

test("basenameFromPath handles unix and windows separators", () => {
  assert.equal(basenameFromPath("/tmp/dir/photo.png"), "photo.png");
  assert.equal(
    basenameFromPath(String.raw`C:\Users\me\photo.png`),
    "photo.png",
  );
  assert.equal(basenameFromPath("/"), "file");
});
