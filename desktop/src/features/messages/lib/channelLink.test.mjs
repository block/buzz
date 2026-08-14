import assert from "node:assert/strict";
import test from "node:test";

import { isChannelLink, parseChannelLink } from "./channelLink.ts";

test("parseChannelLink accepts the canonical channel path", () => {
  assert.deepEqual(
    parseChannelLink("buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32"),
    {
      ok: true,
      value: { channelId: "580ca78b-9dae-46f3-8854-bd671853ba32" },
    },
  );
});

test("parseChannelLink accepts v7 and canonicalizes uppercase UUIDs", () => {
  assert.deepEqual(
    parseChannelLink("buzz://channel/018fdb5d-3a64-7c35-b5f9-4a23e1f9d2d9"),
    {
      ok: true,
      value: { channelId: "018fdb5d-3a64-7c35-b5f9-4a23e1f9d2d9" },
    },
  );
  assert.deepEqual(
    parseChannelLink("buzz://channel/580CA78B-9DAE-46F3-8854-BD671853BA32"),
    {
      ok: true,
      value: { channelId: "580ca78b-9dae-46f3-8854-bd671853ba32" },
    },
  );
});

test("parseChannelLink rejects malformed channel links", () => {
  for (const href of [
    "buzz://channel",
    "buzz://channel/",
    "buzz://channel/one/two",
    "buzz://channel/one?extra=true",
    "buzz://channel/one#fragment",
    "https://channel/one",
    "buzz://channel/not-a-uuid",
    "buzz://channel/%",
    "buzz://channel/%ZZ",
    "buzz://channel/%2F",
    "buzz://channel/%00",
  ]) {
    assert.equal(parseChannelLink(href).ok, false, href);
  }
});

test("isChannelLink recognizes only a valid canonical link", () => {
  assert.equal(
    isChannelLink("buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32"),
    true,
  );
  assert.equal(
    isChannelLink("buzz://message?channel=channel-1&id=message-1"),
    false,
  );
});
