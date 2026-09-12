import assert from "node:assert/strict";
import test from "node:test";

import {
  avatarSourceUrlForShape,
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

test("squareEmojiAvatarDataUrl upgrades known legacy rounded artwork", () => {
  for (const radius of [112, 256]) {
    const legacySvg = `<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512" rx="${radius}" fill="#FFCC00"/><text x="50%" y="56%" dominant-baseline="middle" text-anchor="middle" font-size="258">🐝</text></svg>`;
    const upgraded = squareEmojiAvatarDataUrl(
      `data:image/svg+xml,${encodeURIComponent(legacySvg)}`,
    );
    const svg = decodeSvgDataUrl(upgraded);

    assert.match(svg, /<rect width="512" height="512" fill="#FFCC00"\/>/u);
    assert.doesNotMatch(svg, /\brx=/u);
    assert.match(svg, />🐝<\/text>/u);
  }
});

test("squircle consumers normalize known legacy emoji artwork", () => {
  const legacySvg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512" rx="256" fill="#FFCC00"/><text x="50%" y="56%" dominant-baseline="middle" text-anchor="middle" font-size="258">🐝</text></svg>';
  const legacyUrl = `data:image/svg+xml,${encodeURIComponent(legacySvg)}`;
  const resolved = avatarSourceUrlForShape(legacyUrl, "squircle");
  const svg = decodeSvgDataUrl(resolved);

  assert.doesNotMatch(svg, /\brx=/u);
  assert.match(svg, />🐝<\/text>/u);
  assert.equal(avatarSourceUrlForShape(legacyUrl, "circle"), legacyUrl);
});

test("squareEmojiAvatarDataUrl preserves custom inline SVG artwork", () => {
  const customSvgs = [
    '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><style>text { font-weight: 700; }</style><rect width="512" height="512" fill="#FFCC00"/><path d="M0 0L32 32"/><text x="50%" y="56%" dominant-baseline="middle" text-anchor="middle" font-size="258">Buzz</text></svg>',
    '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512" rx="112" fill="#FFCC00"/><text x="50%" y="56%" dominant-baseline="middle" text-anchor="middle" font-size="258"><tspan fill="red">ACME</tspan></text></svg>',
  ];

  for (const customSvg of customSvgs) {
    const avatarUrl = `data:image/svg+xml,${encodeURIComponent(customSvg)}`;

    assert.equal(parseEmojiAvatarDataUrl(avatarUrl), null);
    assert.equal(squareEmojiAvatarDataUrl(avatarUrl), avatarUrl);
  }
});

test("squareEmojiAvatarDataUrl leaves non-emoji images unchanged", () => {
  const avatarUrl = "https://relay.example/media/avatar.png";
  assert.equal(squareEmojiAvatarDataUrl(avatarUrl), avatarUrl);
});
