import assert from "node:assert/strict";
import { test } from "node:test";

import { mediaProxyUrl } from "./mediaUrl.ts";

const HASH = "a".repeat(64);

test("mediaProxyUrl: uses the IPv4 loopback literal for the localhost proxy", () => {
  assert.equal(
    mediaProxyUrl(54321, `${HASH}.png`),
    `http://127.0.0.1:54321/media/${HASH}.png`,
  );
});
