import assert from "node:assert/strict";
import test from "node:test";

import { KIND_COMMUNITY_SECTIONS } from "@/shared/constants/kinds.ts";

import {
  COMMUNITY_SECTIONS_D_TAG,
  COMMUNITY_SECTIONS_RELAY_UNSUPPORTED_MESSAGE,
  isUnknownCommunitySectionsKindError,
  parseCommunitySectionsPayload,
  selectLatestCommunitySections,
} from "./catalogParse.ts";

test("parseCommunitySectionsPayload accepts valid catalog", () => {
  const sections = parseCommunitySectionsPayload(
    JSON.stringify({
      version: 1,
      sections: [
        {
          id: "eng",
          name: "Engineering",
          order: 1,
          channelIds: ["chan-1", "chan-2"],
        },
        {
          id: "ops",
          name: "Ops",
          order: 0,
          channelIds: ["chan-3"],
        },
      ],
    }),
  );
  assert.equal(sections.length, 2);
  assert.equal(sections[0].id, "ops");
  assert.equal(sections[1].channelIds.length, 2);
});

test("parseCommunitySectionsPayload drops malformed entries", () => {
  const sections = parseCommunitySectionsPayload(
    JSON.stringify({
      version: 1,
      sections: [
        { id: "bad id!", name: "Nope", order: 0, channelIds: [] },
        { id: "ok", name: "Ok", order: 0, channelIds: ["good-1", "bad id"] },
        null,
        "x",
      ],
    }),
  );
  assert.equal(sections.length, 1);
  assert.deepEqual(sections[0].channelIds, ["good-1"]);
});

test("selectLatestCommunitySections picks newest event", () => {
  const older = {
    id: "a",
    pubkey: "p",
    created_at: 10,
    kind: KIND_COMMUNITY_SECTIONS,
    tags: [["d", COMMUNITY_SECTIONS_D_TAG]],
    content: JSON.stringify({
      version: 1,
      sections: [{ id: "old", name: "Old", order: 0, channelIds: [] }],
    }),
    sig: "s",
  };
  const newer = {
    ...older,
    id: "b",
    created_at: 20,
    content: JSON.stringify({
      version: 1,
      sections: [{ id: "new", name: "New", order: 0, channelIds: [] }],
    }),
  };
  const selected = selectLatestCommunitySections([older, newer]);
  assert.equal(selected[0]?.id, "new");
});

test("KIND_COMMUNITY_SECTIONS is 30625", () => {
  assert.equal(KIND_COMMUNITY_SECTIONS, 30625);
  assert.equal(COMMUNITY_SECTIONS_D_TAG, "buzz:community-sections");
});

test("isUnknownCommunitySectionsKindError matches relay OK message", () => {
  assert.equal(
    isUnknownCommunitySectionsKindError(
      new Error("restricted: unknown event kind"),
    ),
    true,
  );
  assert.equal(
    isUnknownCommunitySectionsKindError(new Error("permission denied")),
    false,
  );
  assert.match(COMMUNITY_SECTIONS_RELAY_UNSUPPORTED_MESSAGE, /kind 30625/);
});
