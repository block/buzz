import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

import {
  customEmojiFromEvent,
  customEmojiFromTags,
  normalizeShortcode,
  suggestShortcodeFromFilename,
} from "./customEmoji.ts";

function ev(tags, createdAt = 1) {
  return {
    id: "x",
    pubkey: "relay",
    created_at: createdAt,
    kind: 30030,
    tags,
    content: "",
    sig: "s",
  };
}

test("parses emoji tags into shortcode/url pairs", () => {
  const out = customEmojiFromEvent(
    ev([
      ["d", "buzz:custom-emoji"],
      ["emoji", "party_parrot", "https://relay/p.gif"],
      ["emoji", "shipit", "https://relay/s.png"],
    ]),
  );
  assert.deepEqual(out, [
    { shortcode: "party_parrot", url: "https://relay/p.gif" },
    { shortcode: "shipit", url: "https://relay/s.png" },
  ]);
});

test("null event yields empty list", () => {
  assert.deepEqual(customEmojiFromEvent(null), []);
});

test("skips malformed tags (missing url, bad shortcode chars)", () => {
  const out = customEmojiFromEvent(
    ev([
      ["emoji", "ok", "https://relay/ok.png"],
      ["emoji", "no_url"],
      ["emoji", "bad space", "https://relay/x.png"],
      ["emoji", "", "https://relay/empty.png"],
    ]),
  );
  assert.deepEqual(out, [{ shortcode: "ok", url: "https://relay/ok.png" }]);
});

test("first writer wins on duplicate shortcode", () => {
  const out = customEmojiFromEvent(
    ev([
      ["emoji", "dup", "https://relay/first.png"],
      ["emoji", "dup", "https://relay/second.png"],
    ]),
  );
  assert.deepEqual(out, [{ shortcode: "dup", url: "https://relay/first.png" }]);
});

test("ignores non-emoji tags", () => {
  const out = customEmojiFromEvent(
    ev([
      ["d", "x"],
      ["client", "buzz"],
      ["emoji", "yes", "https://relay/y.png"],
    ]),
  );
  assert.deepEqual(out, [{ shortcode: "yes", url: "https://relay/y.png" }]);
});

test("normalizeShortcode strips colons and lowercases", () => {
  assert.equal(normalizeShortcode(":PartyParrot:"), "partyparrot");
  assert.equal(normalizeShortcode("ship_it"), "ship_it");
  assert.equal(normalizeShortcode("  :Foo-Bar:  "), "foo-bar");
});

test("normalizeShortcode rejects invalid chars and empties", () => {
  assert.equal(normalizeShortcode("bad space"), null);
  assert.equal(normalizeShortcode("emoji!"), null);
  assert.equal(normalizeShortcode("::"), null);
  assert.equal(normalizeShortcode(""), null);
});

test("suggestShortcodeFromFilename derives a valid name from common filenames", () => {
  assert.equal(
    suggestShortcodeFromFilename("Party Parrot.gif"),
    "party_parrot",
  );
  assert.equal(
    suggestShortcodeFromFilename("ship-it.final.PNG"),
    "ship-it_final",
  );
  assert.equal(
    suggestShortcodeFromFilename(path.join("tmp", "Narf! Zort.webp")),
    "narf_zort",
  );
  assert.equal(suggestShortcodeFromFilename("---.png"), null);
});

test("customEmojiFromTags normalizes shortcodes (case-fold)", () => {
  const out = customEmojiFromTags([["emoji", "ShipIt", "https://relay/s.png"]]);
  assert.deepEqual(out, [{ shortcode: "shipit", url: "https://relay/s.png" }]);
});

import { reactionEmojiUrl } from "./customEmoji.ts";

const SET = [{ shortcode: "shipit", url: "https://relay/s.png" }];

test("reactionEmojiUrl resolves :shortcode: against the set (case-insensitive)", () => {
  assert.equal(reactionEmojiUrl(":shipit:", SET), "https://relay/s.png");
  assert.equal(reactionEmojiUrl(":ShipIt:", SET), "https://relay/s.png");
});

test("reactionEmojiUrl returns undefined for unicode / unknown / no set", () => {
  assert.equal(reactionEmojiUrl("👍", SET), undefined);
  assert.equal(reactionEmojiUrl(":nope:", SET), undefined);
  assert.equal(reactionEmojiUrl(":shipit:", undefined), undefined);
});

import { unionCustomEmoji } from "./customEmoji.ts";

test("unionCustomEmoji merges members and sorts by shortcode", () => {
  const out = unionCustomEmoji([
    ev([["emoji", "shipit", "https://relay/s.png"]]),
    ev([["emoji", "ahoy", "https://relay/a.png"]]),
  ]);
  assert.deepEqual(out, [
    { shortcode: "ahoy", url: "https://relay/a.png" },
    { shortcode: "shipit", url: "https://relay/s.png" },
  ]);
});

test("unionCustomEmoji collapses a shortcode to ONE deterministic winner", () => {
  // Two members claim :party_parrot: at the same created_at. The palette must
  // expose exactly one (tie-break: lexicographically-smallest URL), since
  // downstream identity is shortcode-only and cannot disambiguate two URLs.
  const out = unionCustomEmoji([
    ev([["emoji", "party_parrot", "https://relay/zebra.gif"]]),
    ev([["emoji", "party_parrot", "https://relay/alpha.gif"]]),
  ]);
  assert.deepEqual(out, [
    { shortcode: "party_parrot", url: "https://relay/alpha.gif" },
  ]);
});

test("unionCustomEmoji prefers the most recently published set", () => {
  // Newer event wins regardless of URL sort order or input position.
  const older = ev([["emoji", "dup", "https://relay/alpha.gif"]], 100);
  const newer = ev([["emoji", "dup", "https://relay/zebra.gif"]], 200);
  const expected = [{ shortcode: "dup", url: "https://relay/zebra.gif" }];
  assert.deepEqual(unionCustomEmoji([older, newer]), expected);
  assert.deepEqual(unionCustomEmoji([newer, older]), expected);
});

test("unionCustomEmoji winner is independent of member order", () => {
  const a = ev([["emoji", "dup", "https://relay/alpha.gif"]]);
  const b = ev([["emoji", "dup", "https://relay/zebra.gif"]]);
  assert.deepEqual(unionCustomEmoji([a, b]), unionCustomEmoji([b, a]));
});
