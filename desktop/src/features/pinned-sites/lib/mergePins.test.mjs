import assert from "node:assert/strict";
import test from "node:test";

import { mergePinnedSites } from "./mergePins.ts";

test("mergePinnedSites puts community pins first and drops personal id collisions", () => {
  const merged = mergePinnedSites(
    [
      {
        id: "docs",
        name: "My docs",
        url: "https://docs.example.com/",
        icon: "book-open",
        pollForChanges: false,
        openMatchingLinks: true,
        scope: "personal",
      },
      {
        id: "personal",
        name: "Personal",
        url: "https://me.example.com/",
        icon: "house",
        pollForChanges: false,
        openMatchingLinks: true,
        scope: "personal",
      },
    ],
    [
      {
        id: "docs",
        name: "Team docs",
        url: "https://team.example.com/",
        icon: "book-open",
        pollForChanges: true,
        openMatchingLinks: true,
        scope: "community",
      },
    ],
  );

  assert.deepEqual(
    merged.map((pin) => [pin.id, pin.scope, pin.name]),
    [
      ["docs", "community", "Team docs"],
      ["personal", "personal", "Personal"],
    ],
  );
});
