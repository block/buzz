import assert from "node:assert/strict";
import test from "node:test";

import {
  emojiAvatarDataUrl,
  parseEmojiAvatarDataUrl,
  squareEmojiAvatarDataUrl,
} from "./ProfileAvatarEditor.utils.ts";

function decodeSvgDataUrl(dataUrl) {
  return decodeURIComponent(dataUrl.split(",", 2)[1]);
}

test("emojiAvatarDataUrl persists square source artwork", () => {
  const avatarUrl = emojiAvatarDataUrl("✨", "#7657FF");
  const svg = decodeSvgDataUrl(avatarUrl);

  assert.match(svg, /<rect width="512" height="512" fill="#7657FF"\/>/u);
  assert.doesNotMatch(svg, /\brx=/u);
  assert.deepEqual(parseEmojiAvatarDataUrl(avatarUrl), {
    color: "#7657FF",
    emoji: "✨",
  });
});

test("squareEmojiAvatarDataUrl upgrades legacy rounded artwork", () => {
  const legacySvg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512" rx="256" fill="#FFCC00"/><text x="50%" y="56%" dominant-baseline="middle" text-anchor="middle" font-size="258">🐝</text></svg>';
  const upgraded = squareEmojiAvatarDataUrl(
    `data:image/svg+xml,${encodeURIComponent(legacySvg)}`,
  );
  const svg = decodeSvgDataUrl(upgraded);

  assert.match(svg, /<rect width="512" height="512" fill="#FFCC00"\/>/u);
  assert.doesNotMatch(svg, /\brx=/u);
  assert.match(svg, />🐝<\/text>/u);
});

test("squareEmojiAvatarDataUrl leaves non-emoji images unchanged", () => {
  const avatarUrl = "https://relay.example/media/avatar.png";
  assert.equal(squareEmojiAvatarDataUrl(avatarUrl), avatarUrl);
});
