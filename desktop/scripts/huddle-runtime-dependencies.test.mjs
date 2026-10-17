import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

test("Linux installers declare the native huddle video runtime dependencies", () => {
  const config = JSON.parse(
    readFileSync(
      new URL("../src-tauri/tauri.conf.json", import.meta.url),
      "utf8",
    ),
  );
  const dependencies = config.bundle.linux.deb.depends;
  for (const dependency of [
    "gstreamer1.0-pipewire",
    "gstreamer1.0-plugins-good",
    "gstreamer1.0-plugins-bad",
    "gstreamer1.0-libav",
    "xdg-desktop-portal",
  ]) {
    assert.ok(
      dependencies.includes(dependency),
      `Missing runtime: ${dependency}`,
    );
  }
});
