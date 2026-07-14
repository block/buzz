import assert from "node:assert/strict";
import test from "node:test";

import { isMaskedLink } from "./maskedLink.ts";

test("masked when label text differs from destination", () => {
  assert.equal(isMaskedLink("click here", "https://example.com"), true);
  assert.equal(
    isMaskedLink("the docs", "https://docs.example.com/guide"),
    true,
  );
});

test("masked when label is a different URL (spoof)", () => {
  assert.equal(isMaskedLink("https://good.com", "https://evil.com"), true);
  assert.equal(
    isMaskedLink("example.com/safe", "https://example.com/pwn"),
    true,
  );
});

test("not masked when label matches href exactly", () => {
  assert.equal(
    isMaskedLink("https://example.com/a/b", "https://example.com/a/b"),
    false,
  );
});

test("not masked across cosmetic differences", () => {
  // scheme omitted in label
  assert.equal(isMaskedLink("example.com", "https://example.com"), false);
  // trailing slash
  assert.equal(isMaskedLink("example.com", "https://example.com/"), false);
  // GFM autolink of www URLs prefixes http://
  assert.equal(
    isMaskedLink("www.example.com", "http://www.example.com"),
    false,
  );
  // case-insensitive
  assert.equal(isMaskedLink("Example.COM", "https://example.com"), false);
  // surrounding whitespace in label
  assert.equal(isMaskedLink(" example.com ", "https://example.com"), false);
});

test("not masked when there is nothing useful to compare", () => {
  assert.equal(isMaskedLink("", "https://example.com"), false);
  assert.equal(isMaskedLink("   ", "https://example.com"), false);
  assert.equal(isMaskedLink("label", ""), false);
});

test("non-http schemes are never masked", () => {
  assert.equal(isMaskedLink("email me", "mailto:a@b.com"), false);
  assert.equal(
    isMaskedLink("open thread", "buzz://message?channel=x&id=y"),
    false,
  );
});

test("masked on scheme downgrade even when host matches", () => {
  assert.equal(
    isMaskedLink("https://accounts.example.com", "http://accounts.example.com"),
    true,
  );
});

test("not masked when label repeats href scheme exactly", () => {
  assert.equal(isMaskedLink("http://example.com", "http://example.com"), false);
});

test("masked when path or query differ only by case", () => {
  assert.equal(
    isMaskedLink(
      "https://example.com/Reset?token=ABC123",
      "https://example.com/reset?token=abc123",
    ),
    true,
  );
});

test("host case differences alone are not masked", () => {
  assert.equal(
    isMaskedLink("Example.COM/path", "https://example.com/path"),
    false,
  );
});

test("protocol-relative hrefs are covered", () => {
  assert.equal(isMaskedLink("company wiki", "//evil.example/login"), true);
  assert.equal(
    isMaskedLink("//evil.example/login", "//evil.example/login"),
    false,
  );
});

test("labels with userinfo are always masked", () => {
  // Visually reads as safe.com but actually resolves to evil.com
  assert.equal(isMaskedLink("safe.com@evil.com", "https://evil.com"), true);
  assert.equal(
    isMaskedLink("https://safe.com@evil.com", "https://evil.com"),
    true,
  );
});
