import assert from "node:assert/strict";
import test from "node:test";

import { channelLifecycle, channelLifecycleLabel } from "./channelLifecycle.ts";

test("channelLifecycle prefers project home over TTL", () => {
  assert.equal(
    channelLifecycle({ projectHome: true, temporary: false }),
    "project",
  );
  assert.equal(
    channelLifecycle({ projectHome: true, temporary: true }),
    "project",
  );
});

test("channelLifecycle maps ongoing and temporary streams", () => {
  assert.equal(
    channelLifecycle({ projectHome: false, temporary: false }),
    "ongoing",
  );
  assert.equal(
    channelLifecycle({ projectHome: false, temporary: true }),
    "temporary",
  );
});

test("channelLifecycleLabel names project, ongoing, and temporary", () => {
  assert.equal(channelLifecycleLabel("project", null), "Project");
  assert.equal(channelLifecycleLabel("ongoing", null), "Ongoing");
  assert.equal(
    channelLifecycleLabel("temporary", 7 * 24 * 60 * 60),
    "Temporary · 7d",
  );
});
