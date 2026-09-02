import assert from "node:assert/strict";
import test from "node:test";

import { connectHermesProfiles } from "./connectHermesProfiles.ts";

test("connects profiles with bounded concurrency and preserves partial failures", async () => {
  let active = 0;
  let maxActive = 0;
  const completed = [];
  const profiles = ["jake", "archie", "daedalus", "hestia"];

  const result = await connectHermesProfiles({
    profiles,
    concurrency: 2,
    connect: async (profile) => {
      active += 1;
      maxActive = Math.max(maxActive, active);
      await new Promise((resolve) => setTimeout(resolve, 5));
      active -= 1;
      if (profile === "daedalus") throw new Error("provider unavailable");
      completed.push(profile);
      return `${profile}-pubkey`;
    },
  });

  assert.equal(maxActive, 2);
  assert.deepEqual(completed.sort(), ["archie", "hestia", "jake"]);
  assert.deepEqual(result.successes.map((entry) => entry.profile).sort(), [
    "archie",
    "hestia",
    "jake",
  ]);
  assert.deepEqual(result.failures, [
    { profile: "daedalus", error: "provider unavailable" },
  ]);
});

test("empty selection performs no work", async () => {
  let calls = 0;
  const result = await connectHermesProfiles({
    profiles: [],
    connect: async () => {
      calls += 1;
      return "never";
    },
  });
  assert.equal(calls, 0);
  assert.deepEqual(result, { successes: [], failures: [] });
});
