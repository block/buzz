import assert from "node:assert/strict";
import test from "node:test";

import { initializeI18n } from "@/i18n/index.ts";
import { channelLifecycle, channelLifecycleLabel } from "./channelLifecycle.ts";

// `channelLifecycleLabel` resolves its text through `i18n.t`, which returns
// `undefined` until the singleton boots (`main.tsx` does that in the app).
// English is pinned explicitly before init: node's own `navigator.languages`
// reports the host system locale — which may be zh-CN — and every assertion
// below is the English contract.
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});
initializeI18n();

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
  assert.equal(channelLifecycleLabel("temporary", null), "Temporary");
});
