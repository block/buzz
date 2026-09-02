import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";

import {
  channelProjectFeatureEnabled,
  findChannelProject,
  parseChannelProjectFeatureStore,
  projectPrimaryRepository,
  projectRelatedChannelIds,
  projectRelatedRepositories,
  readChannelProjectFeaturePreferences,
  writeChannelProjectFeaturePreferences,
} from "./channelProjectFeatures.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  globalThis.window = dom.window;
});
beforeEach(() => dom.window.localStorage.clear());
after(() => dom.window.close());

test("feature preferences are scoped by viewer, relay, and channel", () => {
  writeChannelProjectFeaturePreferences(
    "viewer-a",
    "wss://relay-a.example/",
    "channel-a",
    { reviews: true, tasks: true },
  );

  assert.deepEqual(
    readChannelProjectFeaturePreferences(
      "viewer-a",
      "wss://relay-a.example",
      "channel-a",
    ),
    { reviews: true, tasks: true },
  );
  assert.deepEqual(
    readChannelProjectFeaturePreferences(
      "viewer-b",
      "wss://relay-a.example",
      "channel-a",
    ),
    {},
  );
  assert.deepEqual(
    readChannelProjectFeaturePreferences(
      "viewer-a",
      "wss://relay-b.example",
      "channel-a",
    ),
    {},
  );
  assert.deepEqual(
    readChannelProjectFeaturePreferences(
      "viewer-a",
      "wss://relay-a.example",
      "channel-b",
    ),
    {},
  );
});

test("malformed feature storage fails closed", () => {
  assert.deepEqual(parseChannelProjectFeatureStore(null), {
    version: 1,
    channels: {},
  });
  assert.deepEqual(
    parseChannelProjectFeatureStore({
      version: 1,
      channels: {
        valid: { reviews: true, tasks: true, repositories: "yes" },
        empty: null,
      },
    }),
    {
      version: 1,
      channels: { valid: { reviews: true, tasks: true }, empty: {} },
    },
  );
});

test("existing data keeps a locally disabled feature enabled", () => {
  assert.equal(
    channelProjectFeatureEnabled({
      feature: "tasks",
      hasExistingData: true,
      preferences: { tasks: false },
    }),
    true,
  );
  assert.equal(
    channelProjectFeatureEnabled({
      feature: "tasks",
      hasExistingData: false,
      preferences: { tasks: false },
    }),
    false,
  );
});

test("channel project helpers hide the primary repository and dedupe breakout channels", () => {
  const primary = {
    id: "primary",
    repoAddress: "30617:owner:primary",
    channelId: "root",
  };
  const related = {
    id: "related",
    repoAddress: "30617:owner:related",
    channelId: "breakout",
  };
  const project = {
    id: "project",
    legacy: false,
    projectChannelId: "root",
    primaryRepositoryAddress: primary.repoAddress,
    relatedChannelIds: ["breakout", "extra", "root"],
    repositories: [primary, related, { ...related, id: "duplicate" }],
  };

  assert.equal(findChannelProject([project], "root"), project);
  assert.equal(projectPrimaryRepository(project), primary);
  assert.deepEqual(projectRelatedRepositories(project), [
    related,
    {
      ...related,
      id: "duplicate",
    },
  ]);
  assert.deepEqual(projectRelatedChannelIds(project, "root"), [
    "breakout",
    "extra",
  ]);
});
