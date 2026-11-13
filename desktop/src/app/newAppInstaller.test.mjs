import assert from "node:assert/strict";
import test from "node:test";

import { resolveInstallerUrl } from "./newAppInstaller.ts";

const RELEASE = "https://github.com/block/buzz-app/releases/download/v1.0.0";
const DOWNLOADS = {
  "macos-arm64": `${RELEASE}/Buzz_1.0.0_aarch64.dmg`,
  "macos-x64": `${RELEASE}/Buzz_1.0.0_x64.dmg`,
  "windows-x64": `${RELEASE}/Buzz_1.0.0_x64_unsigned.exe`,
  "linux-appimage": `${RELEASE}/Buzz_1.0.0_x86_64.AppImage`,
  "linux-deb": `${RELEASE}/Buzz_1.0.0_amd64.deb`,
};

function manifest(overrides = {}) {
  return {
    schemaVersion: 1,
    channel: "stable",
    version: "1.0.0",
    release: "v1.0.0",
    downloads: DOWNLOADS,
    ...overrides,
  };
}

function withDownload(url) {
  return manifest({ downloads: { ...DOWNLOADS, "macos-arm64": url } });
}

test("resolves the installer for every shipped target", () => {
  for (const [target, url] of Object.entries(DOWNLOADS)) {
    assert.equal(resolveInstallerUrl(manifest(), target), url, target);
  }
});

test("rejects every manifest or entry that is not a buzz-app release asset", () => {
  const cases = [
    ["manifest is not an object", "not json", "macos-arm64"],
    ["manifest is null", null, "macos-arm64"],
    ["newer schema version", manifest({ schemaVersion: 2 }), "macos-arm64"],
    [
      "schema version as a string",
      manifest({ schemaVersion: "1" }),
      "macos-arm64",
    ],
    ["other channel", manifest({ channel: "beta" }), "macos-arm64"],
    ["downloads missing", manifest({ downloads: undefined }), "macos-arm64"],
    [
      "target missing from downloads",
      manifest({ downloads: { "macos-x64": DOWNLOADS["macos-x64"] } }),
      "macos-arm64",
    ],
    ["target unknown to this app", manifest(), "linux-arm64"],
    ["target that is an inherited property", manifest(), "constructor"],
    ["entry is not a string", withDownload(42), "macos-arm64"],
    ["entry is not a URL", withDownload("Buzz.dmg"), "macos-arm64"],
    [
      "plain http",
      withDownload(DOWNLOADS["macos-arm64"].replace("https:", "http:")),
      "macos-arm64",
    ],
    [
      "another host",
      withDownload(
        "https://example.org/block/buzz-app/releases/download/v1.0.0/Buzz.dmg",
      ),
      "macos-arm64",
    ],
    [
      "host hidden behind userinfo",
      withDownload(
        "https://github.com@example.org/block/buzz-app/releases/download/v1.0.0/Buzz.dmg",
      ),
      "macos-arm64",
    ],
    [
      "credentials in the URL",
      withDownload(
        "https://user:pass@github.com/block/buzz-app/releases/download/v1.0.0/Buzz.dmg",
      ),
      "macos-arm64",
    ],
    [
      "non-default port",
      withDownload(
        "https://github.com:8443/block/buzz-app/releases/download/v1.0.0/Buzz.dmg",
      ),
      "macos-arm64",
    ],
    [
      "another repository",
      withDownload(
        "https://github.com/example-org/buzz-app/releases/download/v1.0.0/Buzz.dmg",
      ),
      "macos-arm64",
    ],
    [
      "dot segments that leave the repository",
      withDownload(
        "https://github.com/block/buzz-app/releases/download/../../../../example-org/other/releases/download/v1/Buzz.dmg",
      ),
      "macos-arm64",
    ],
    [
      "encoded dot segments that leave the repository",
      withDownload(
        "https://github.com/block/buzz-app/releases/download/%2e%2e/%2e%2e/%2e%2e/%2e%2e/example-org/other/Buzz.dmg",
      ),
      "macos-arm64",
    ],
    [
      "wrong file type for the target",
      withDownload(DOWNLOADS["windows-x64"]),
      "macos-arm64",
    ],
    [
      "expected extension only in the query",
      withDownload(`${RELEASE}/Buzz_1.0.0_x64_unsigned.exe?name=Buzz.dmg`),
      "macos-arm64",
    ],
    ["query", withDownload(`${RELEASE}/Buzz.dmg?download=1`), "macos-arm64"],
    ["empty query", withDownload(`${RELEASE}/Buzz.dmg?`), "macos-arm64"],
    ["fragment", withDownload(`${RELEASE}/Buzz.dmg#top`), "macos-arm64"],
    [
      "no tag segment",
      withDownload(
        "https://github.com/block/buzz-app/releases/download/Buzz.dmg",
      ),
      "macos-arm64",
    ],
    // The tag reaches GitHub as written, so nothing in it may be an escape
    // GitHub's router could decode into another path.
    ...[
      "%2F..%2F..%2Fother",
      "%5Cother",
      "%00",
      ";other",
      "%20beta",
      " beta",
      "@other",
    ].map((suffix) => [
      `tag ending in ${suffix}`,
      withDownload(`${RELEASE}${suffix}/Buzz.dmg`),
      "macos-arm64",
    ]),
    [
      "extension with nothing before it",
      withDownload(`${RELEASE}/.dmg`),
      "macos-arm64",
    ],
    ["hidden file", withDownload(`${RELEASE}/.Buzz.dmg`), "macos-arm64"],
    [
      "encoded separator in the file name",
      withDownload(`${RELEASE}/..%2FBuzz.dmg`),
      "macos-arm64",
    ],
    [
      "space in the file name",
      withDownload(`${RELEASE}/Buzz%20App.dmg`),
      "macos-arm64",
    ],
    [
      "file name too long",
      withDownload(`${RELEASE}/${"a".repeat(128)}.dmg`),
      "macos-arm64",
    ],
  ];
  for (const [name, input, target] of cases) {
    assert.equal(resolveInstallerUrl(input, target), null, name);
  }
});

test("accepts a tag of slashes and the characters release tags use", () => {
  for (const tag of ["desktop/v1.0.0", "desktop/v1.0.0-rc.1+build_2"]) {
    const url = `https://github.com/block/buzz-app/releases/download/${tag}/Buzz.dmg`;
    assert.equal(resolveInstallerUrl(withDownload(url), "macos-arm64"), url);
  }
});
