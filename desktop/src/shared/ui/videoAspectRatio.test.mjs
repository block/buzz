import assert from "node:assert/strict";
import test from "node:test";

import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  getRememberedVideoAspectRatio,
  rememberVideoAspectRatio,
  useNaturalVideoAspectRatio,
} from "./videoAspectRatio.ts";

// The virtualized timeline evicts and remounts video rows. VideoPlayer reads
// `useNaturalVideoAspectRatio`, whose state seed is this module's cache — the
// contract that keeps a remounted dim-less video at its true height is: a
// ratio learned on first `loadedmetadata` must seed a later mount of the same
// src, so the row never falls back to the 16/9 placeholder again.

// Renders the hook's current ratio. Each renderToStaticMarkup call is a fresh
// mount — exactly what a retention eviction/remount does to the real player.
function MountedRatio({ src }) {
  const [ratio] = useNaturalVideoAspectRatio(src);
  return React.createElement("span", null, ratio === null ? "none" : ratio);
}

function mountRatio(src) {
  return renderToStaticMarkup(React.createElement(MountedRatio, { src }));
}

test("a learned non-16:9 ratio survives remount", () => {
  const src = "https://relay.example/media/portrait.mp4";
  const portrait = 9 / 16;

  // First mount: nothing learned yet — the player can only fall back.
  assert.equal(mountRatio(src), "<span>none</span>");

  // Metadata arrives on the first mount; the player learns the true ratio.
  rememberVideoAspectRatio(src, portrait);

  // Fresh mount of the same src (state reset, as after retention eviction):
  // the hook seeds from the cache instead of the 16/9 fallback path.
  assert.equal(mountRatio(src), `<span>${portrait}</span>`);
});

test("different srcs do not share learned ratios", () => {
  rememberVideoAspectRatio("https://relay.example/media/a.mp4", 1);
  assert.equal(
    mountRatio("https://relay.example/media/b.mp4"),
    "<span>none</span>",
  );
});

test("invalid ratios and missing srcs are ignored", () => {
  rememberVideoAspectRatio(undefined, 1.5);
  rememberVideoAspectRatio("https://relay.example/media/bad.mp4", 0);
  rememberVideoAspectRatio("https://relay.example/media/bad.mp4", Number.NaN);
  assert.equal(
    getRememberedVideoAspectRatio("https://relay.example/media/bad.mp4"),
    null,
  );
  assert.equal(getRememberedVideoAspectRatio(undefined), null);
});
