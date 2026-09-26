import assert from "node:assert/strict";
import test from "node:test";

import { KIND_COMMUNITY_PINNED_SITES } from "@/shared/constants/kinds.ts";

import {
  parseCommunityPinnedSitesPayload,
  parseCommunityPinnedSitesWithRunbooks,
  selectLatestCommunityPins,
} from "./communityPins.ts";

test("parseCommunityPinnedSitesPayload keeps valid https pins", () => {
  const pins = parseCommunityPinnedSitesPayload(
    JSON.stringify({
      version: 1,
      pins: [
        {
          id: "docs",
          name: "Docs",
          url: "https://example.com/docs",
          icon: "book-open",
          pollForChanges: true,
        },
        {
          id: "bad",
          name: "Bad",
          url: "http://example.com",
          icon: "globe",
        },
      ],
    }),
  );
  assert.equal(pins.length, 1);
  assert.equal(pins[0].id, "docs");
  assert.equal(pins[0].scope, "community");
  assert.equal(pins[0].pollForChanges, true);
  assert.equal(pins[0].openMatchingLinks, true);
});

test("selectLatestCommunityPins uses the newest created_at", () => {
  const pins = selectLatestCommunityPins([
    {
      id: "older",
      kind: KIND_COMMUNITY_PINNED_SITES,
      created_at: 10,
      content: JSON.stringify({
        version: 1,
        pins: [
          {
            id: "old",
            name: "Old",
            url: "https://old.example/",
            icon: "globe",
          },
        ],
      }),
    },
    {
      id: "newer",
      kind: KIND_COMMUNITY_PINNED_SITES,
      created_at: 20,
      content: JSON.stringify({
        version: 1,
        pins: [
          { id: "new", name: "New", url: "https://new.example/", icon: "star" },
        ],
      }),
    },
  ]);
  assert.equal(pins.length, 1);
  assert.equal(pins[0].id, "new");
});

test("parseCommunityPinnedSitesPayload respects openMatchingLinks false", () => {
  const pins = parseCommunityPinnedSitesPayload(
    JSON.stringify({
      version: 1,
      pins: [
        {
          id: "docs",
          name: "Docs",
          url: "https://example.com/docs",
          icon: "book-open",
          openMatchingLinks: false,
        },
      ],
    }),
  );
  assert.equal(pins.length, 1);
  assert.equal(pins[0].openMatchingLinks, false);
});


test("community pin runbook payload hydrates active procedures", () => {
  const rows = parseCommunityPinnedSitesWithRunbooks(
    JSON.stringify({
      version: 1,
      pins: [
        {
          id: "wf",
          name: "Wayfinder",
          url: "https://wayfinder.huladesk.com",
          icon: "compass",
          runbook: {
            agentBrief: "Prefer left nav",
            procedures: [
              {
                id: "p1",
                title: "Open project",
                steps: "Click Projects",
                createdAt: 1,
                updatedAt: 2,
              },
            ],
          },
        },
      ],
    }),
  );
  assert.equal(rows.length, 1);
  assert.equal(rows[0].pin.id, "wf");
  assert.equal(rows[0].runbook?.agentBrief, "Prefer left nav");
  assert.equal(rows[0].runbook?.procedures[0]?.status, "active");
  assert.equal(rows[0].runbook?.procedures[0]?.title, "Open project");
});
