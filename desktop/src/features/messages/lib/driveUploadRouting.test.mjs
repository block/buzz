import assert from "node:assert/strict";
import { test } from "node:test";

import {
  DRIVE_UPLOAD_THRESHOLD_BYTES,
  isAudioFile,
  isRelayBlockedFile,
  isRelayUnavailableError,
  uploadRouteFor,
} from "./driveUploadRouting.mjs";

const MB = 1024 * 1024;

// --- the size rule ----------------------------------------------------------

test("a small ordinary file stays on the relay", () => {
  assert.equal(
    uploadRouteFor({
      name: "notes.pdf",
      type: "application/pdf",
      sizeBytes: 2 * MB,
    }),
    "relay",
  );
});

test("anything over the threshold goes to Drive", () => {
  assert.equal(
    uploadRouteFor({
      name: "deck.pptx",
      type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      sizeBytes: DRIVE_UPLOAD_THRESHOLD_BYTES + 1,
    }),
    "drive",
  );
});

test("exactly the threshold stays on the relay", () => {
  assert.equal(
    uploadRouteFor({
      name: "deck.pptx",
      type: "application/pdf",
      sizeBytes: DRIVE_UPLOAD_THRESHOLD_BYTES,
    }),
    "relay",
  );
});

test("the threshold is the point Drive's simple upload stops working", () => {
  assert.equal(DRIVE_UPLOAD_THRESHOLD_BYTES, 5 * MB);
});

// --- the media rule ---------------------------------------------------------

test("a tiny video still goes to Drive", () => {
  assert.equal(
    uploadRouteFor({
      name: "clip.mp4",
      type: "video/mp4",
      sizeBytes: 1024,
      isVideo: true,
    }),
    "drive",
  );
});

test("a tiny audio file still goes to Drive", () => {
  assert.equal(
    uploadRouteFor({ name: "note.m4a", type: "audio/mp4", sizeBytes: 1024 }),
    "drive",
  );
});

test("audio is detected by extension when the MIME type says nothing", () => {
  assert.ok(isAudioFile({ name: "voice.mp3", type: "" }));
  assert.ok(
    isAudioFile({ name: "voice.wav", type: "application/octet-stream" }),
  );
  assert.ok(isAudioFile({ name: "voice.OGG" }));
});

test("a concrete MIME type outranks a misleading extension", () => {
  // A PDF named .mp3 is a PDF, and small enough to stay on the relay.
  assert.ok(!isAudioFile({ name: "report.mp3", type: "application/pdf" }));
  assert.equal(
    uploadRouteFor({
      name: "report.mp3",
      type: "application/pdf",
      sizeBytes: 1024,
    }),
    "relay",
  );
});

test("a file with no extension and no MIME type is not audio", () => {
  assert.ok(!isAudioFile({ name: "README" }));
  assert.ok(!isAudioFile({}));
});

// --- executables, which the relay genuinely rejects ---------------------------

test("every blocked MIME type routes to Drive at any size", () => {
  const types = [
    "application/x-msdownload",
    "application/vnd.microsoft.portable-executable",
    "application/x-executable",
    "application/x-sharedlib",
    "application/x-elf",
    "application/x-mach-binary",
  ];
  for (const type of types) {
    assert.ok(isRelayBlockedFile({ name: "thing", type }), type);
    assert.equal(
      uploadRouteFor({ name: "thing", type, sizeBytes: 1024 }),
      "drive",
      type,
    );
  }
});

test("executables are detected by extension when the MIME says nothing", () => {
  const names = [
    "setup.exe",
    "driver.dll",
    "kernel.sys",
    "screensaver.scr",
    "legacy.COM",
    "lib.so",
    "module.ko",
    "object.o",
    "shared.dylib",
    "plugin.bundle",
  ];
  for (const name of names) {
    assert.ok(isRelayBlockedFile({ name, type: "" }), name);
    assert.ok(
      isRelayBlockedFile({ name, type: "application/octet-stream" }),
      name,
    );
  }
});

// The relay sniffs *content* with the `infer` crate, which has no matcher for
// any of these, so they upload fine today. Routing them to Drive would break a
// working flow for anyone without a Google account connected. See the comment
// on RELAY_BLOCKED_MIME_TYPES.
test("types the relay actually accepts are NOT routed to Drive", () => {
  const cases = [
    ["logo.svg", "image/svg+xml"],
    ["app.js", "application/javascript"],
    ["mod.mjs", "text/javascript"],
    ["page.xhtml", "application/xhtml+xml"],
    ["installer.msi", "application/x-msi"],
    ["build.apk", "application/vnd.android.package-archive"],
    ["disk.dmg", "application/x-apple-diskimage"],
    ["report.html", "text/html"],
  ];
  for (const [name, type] of cases) {
    assert.ok(!isRelayBlockedFile({ name, type }), name);
    assert.equal(
      uploadRouteFor({ name, type, sizeBytes: 2048 }),
      "relay",
      name,
    );
  }
});

test("those same types still go to Drive once they are large", () => {
  assert.equal(
    uploadRouteFor({
      name: "logo.svg",
      type: "image/svg+xml",
      sizeBytes: 40 * MB,
    }),
    "drive",
  );
});

test("a concrete safe MIME type beats a blocked-looking extension", () => {
  // A PNG named .exe is a PNG. The relay sniffs content and accepts it.
  assert.ok(!isRelayBlockedFile({ name: "chart.exe", type: "image/png" }));
  assert.equal(
    uploadRouteFor({ name: "chart.exe", type: "image/png", sizeBytes: 2048 }),
    "relay",
  );
});

test("a blocked MIME type beats an innocuous extension", () => {
  assert.ok(
    isRelayBlockedFile({ name: "logo.png", type: "application/x-mach-binary" }),
  );
});

test("a file with neither name nor type is not treated as blocked", () => {
  assert.ok(!isRelayBlockedFile({}));
  assert.ok(!isRelayBlockedFile({ name: "README" }));
});

test("an extensionless binary is a known, accepted gap", () => {
  // ELF/Mach-O binaries usually have no extension and browsers give them no
  // MIME type, so we cannot see them. They take the relay path and get the
  // rejection they get today - no regression, just no improvement.
  assert.ok(!isRelayBlockedFile({ name: "myprogram", type: "" }));
});

test("an image under the threshold still goes to the relay", () => {
  assert.equal(
    uploadRouteFor({
      name: "screenshot.png",
      type: "image/png",
      sizeBytes: 2048,
    }),
    "relay",
  );
});

test("a missing or unparseable size does not force the Drive path", () => {
  assert.equal(uploadRouteFor({ name: "a.txt", type: "text/plain" }), "relay");
  assert.equal(
    uploadRouteFor({
      name: "a.txt",
      type: "text/plain",
      sizeBytes: Number.NaN,
    }),
    "relay",
  );
});

// --- the 5xx-unavailable fallback trigger -----------------------------------

test("a relay 5xx availability error triggers the Drive fallback", () => {
  for (const status of [500, 502, 503, 504, 599]) {
    assert.equal(
      isRelayUnavailableError(
        new Error(`relay returned ${status} Service Unavailable`),
      ),
      true,
      `status ${status}`,
    );
  }
});

test("a 4xx client error does not trigger the fallback", () => {
  for (const status of [400, 401, 403, 413, 415, 429]) {
    assert.equal(
      isRelayUnavailableError(new Error(`relay returned ${status}`)),
      false,
      `status ${status}`,
    );
  }
});

test("a non-HTTP failure does not trigger the fallback", () => {
  assert.equal(isRelayUnavailableError(new Error("upload cancelled")), false);
  assert.equal(
    isRelayUnavailableError(new Error("error sending request")),
    false,
  );
  assert.equal(isRelayUnavailableError(undefined), false);
  assert.equal(isRelayUnavailableError(null), false);
});

test("a plain string 503 is accepted too", () => {
  assert.equal(
    isRelayUnavailableError("relay returned 503 Service Unavailable"),
    true,
  );
});
