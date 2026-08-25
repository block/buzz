import assert from "node:assert/strict";
import { test } from "node:test";
import { createCodexLabLatestManifest } from "./generate-codex-lab-latest.mjs";

test("creates a signed Windows updater manifest", () => {
  const manifest = createCodexLabLatestManifest({
    version: "0.5.12-test.3",
    signature: "  trusted-signature\n",
    url: "https://github.com/chemyibinjiang/buzz/releases/download/buzz-codex-lab-v0.5.12-test.3/Buzz.exe",
    pubDate: "2026-08-25T06:00:00.000Z",
  });

  assert.deepEqual(manifest, {
    version: "0.5.12-test.3",
    notes: "Buzz Codex Lab v0.5.12-test.3",
    pub_date: "2026-08-25T06:00:00.000Z",
    platforms: {
      "windows-x86_64": {
        signature: "trusted-signature",
        url: "https://github.com/chemyibinjiang/buzz/releases/download/buzz-codex-lab-v0.5.12-test.3/Buzz.exe",
      },
    },
  });
});

test("rejects insecure artifact URLs", () => {
  assert.throws(
    () =>
      createCodexLabLatestManifest({
        version: "0.5.12",
        signature: "signature",
        url: "http://10.24.11.82/Buzz.exe",
      }),
    /must use HTTPS/,
  );
});

test("allows insecure artifact URLs only when explicitly enabled", () => {
  const manifest = createCodexLabLatestManifest({
    version: "0.5.12",
    signature: "signature",
    url: "http://10.24.11.82/Buzz.exe",
    allowInsecure: true,
  });

  assert.equal(manifest.platforms["windows-x86_64"].url, "http://10.24.11.82/Buzz.exe");
});

test("rejects malformed versions and empty signatures", () => {
  assert.throws(
    () =>
      createCodexLabLatestManifest({
        version: "latest",
        signature: "signature",
        url: "https://example.test/Buzz.exe",
      }),
    /version must be semantic/,
  );
  assert.throws(
    () =>
      createCodexLabLatestManifest({
        version: "0.5.12",
        signature: " ",
        url: "https://example.test/Buzz.exe",
      }),
    /signature must not be empty/,
  );
});
