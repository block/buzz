import assert from "node:assert/strict";
import test from "node:test";

import { extractSupportedLinkPreviews } from "../../lib/linkPreview.ts";
import { parseLinkPreviewSnapshots } from "../../lib/linkPreviewSnapshot.ts";
import { mergeMessageLinkPreviews } from "./useMessageLinkPreviews.ts";

const OWNER = "a".repeat(64);
const EVENT_ID = "b".repeat(64);
const ENTITY_HREF = `buzz://pr?id=${EVENT_ID}&owner=${OWNER}&d=buzz-world`;
const EXTERNAL_HREF = "https://example.com/story";
const RELAY_ORIGIN = "https://relay.example";

function snapshotTag(href, title, siteName) {
  return [
    "link-preview",
    "snapshot",
    "1",
    href,
    title,
    siteName,
    "",
    "",
    "",
    "",
    "",
  ];
}

test("relay-resolved Buzz entities beat conflicting sender snapshots in content order", () => {
  const content = `${ENTITY_HREF} then ${EXTERNAL_HREF}`;
  const candidates = extractSupportedLinkPreviews(content, RELAY_ORIGIN);
  const snapshots = parseLinkPreviewSnapshots(
    [
      snapshotTag(ENTITY_HREF, "Forged sender title", "Definitely Real Buzz"),
      snapshotTag(EXTERNAL_HREF, "External story", "Example"),
    ],
    content,
    RELAY_ORIGIN,
  );
  const entity = {
    ...candidates[0],
    title: "Relay-authenticated PR title",
    imageState: "none",
  };

  assert.deepEqual(
    mergeMessageLinkPreviews(candidates, snapshots, [entity]).map(
      ({ href, title, provider }) => ({ href, title, provider }),
    ),
    [
      {
        href: ENTITY_HREF,
        title: "Relay-authenticated PR title",
        provider: "Buzz",
      },
      {
        href: EXTERNAL_HREF,
        title: "External story",
        provider: "Example",
      },
    ],
  );
});
