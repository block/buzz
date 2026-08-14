import assert from "node:assert/strict";
import test from "node:test";

import { nip19 } from "nostr-tools";

import {
  isAllowedComposerLink,
  shouldAutoLinkComposerUrl,
} from "./composerLinks.ts";

const PUBKEY = "1".repeat(64);
const VALID_NADDR = `nostr:${nip19.naddrEncode({
  identifier: "article",
  kind: 30023,
  pubkey: PUBKEY,
})}`;
const NSEC = `nostr:${nip19.nsecEncode(new Uint8Array(32).fill(1))}`;

test("composer link validation accepts only supported nostr references", () => {
  assert.equal(
    isAllowedComposerLink(VALID_NADDR, () => false),
    true,
  );
  assert.equal(
    isAllowedComposerLink(NSEC, () => true),
    false,
  );
});

test("composer autolinking accepts only supported nostr references", () => {
  assert.equal(
    shouldAutoLinkComposerUrl(VALID_NADDR, () => false),
    true,
  );
  assert.equal(
    shouldAutoLinkComposerUrl(NSEC, () => true),
    false,
  );
});

test("composer link policies delegate non-nostr URLs to TipTap", () => {
  assert.equal(
    isAllowedComposerLink("https://example.com", () => true),
    true,
  );
  assert.equal(
    shouldAutoLinkComposerUrl("https://example.com", () => false),
    false,
  );
});
