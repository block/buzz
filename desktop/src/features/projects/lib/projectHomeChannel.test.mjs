import assert from "node:assert/strict";
import test from "node:test";

import {
  hasAuthoritativeHomeBinding,
  isProjectHomeChannel,
} from "./projectHomeChannel.ts";

const OWNER = "a".repeat(64);
const MAINTAINER = "b".repeat(64);

function project(overrides = {}) {
  return {
    owner: OWNER,
    projectChannelId: "channel-a",
    repositories: [
      {
        channelId: "channel-a",
        owner: OWNER,
      },
    ],
    ...overrides,
  };
}

test("isProjectHomeChannel accepts an owner-bound repository", () => {
  assert.equal(isProjectHomeChannel("channel-a", [project()]), true);
});

test("isProjectHomeChannel accepts a repository that authorizes the project owner", () => {
  assert.equal(
    isProjectHomeChannel("channel-a", [
      project({
        owner: MAINTAINER.toUpperCase(),
        repositories: [
          {
            channelId: "channel-a",
            maintainers: [OWNER, MAINTAINER],
            owner: OWNER,
          },
        ],
      }),
    ]),
    true,
  );
});

test("hasAuthoritativeHomeBinding rejects a bare project route assertion", () => {
  assert.equal(
    hasAuthoritativeHomeBinding(project({ repositories: [] })),
    false,
  );
});

test("isProjectHomeChannel rejects a bare project channel assertion", () => {
  assert.equal(
    isProjectHomeChannel("channel-a", [project({ repositories: [] })]),
    false,
  );
});

test("isProjectHomeChannel rejects unauthorized and mismatched repository bindings", () => {
  assert.equal(
    isProjectHomeChannel("channel-a", [
      project({
        owner: MAINTAINER,
        repositories: [{ channelId: "channel-a", owner: OWNER }],
      }),
      project({
        repositories: [{ channelId: "channel-b", owner: OWNER }],
      }),
    ]),
    false,
  );
});

test("isProjectHomeChannel is false for unbound channels", () => {
  assert.equal(isProjectHomeChannel("channel-z", [project()]), false);
  assert.equal(isProjectHomeChannel(null, [project()]), false);
});
