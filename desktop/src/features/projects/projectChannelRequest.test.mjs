import assert from "node:assert/strict";
import test from "node:test";

import {
  parseProjectChannelRequest,
  PROJECT_CHANNEL_REQUEST,
} from "./projectChannelRequest.ts";

const HOME_CHANNEL = "11111111-1111-4111-8111-111111111111";

test("parses a narrow project channel request", () => {
  assert.deepEqual(
    parseProjectChannelRequest({
      type: PROJECT_CHANNEL_REQUEST,
      action: "create",
      requestId: "request-1",
      request: {
        homeChannelId: HOME_CHANNEL,
        name: "release-planning",
        description: "Coordinate the release.",
        visibility: "private",
        ttlSeconds: 3600,
        templateName: "Release team",
      },
    }),
    {
      type: PROJECT_CHANNEL_REQUEST,
      action: "create",
      requestId: "request-1",
      request: {
        homeChannelId: HOME_CHANNEL,
        name: "release-planning",
        description: "Coordinate the release.",
        visibility: "private",
        ttlSeconds: 3600,
        templateName: "Release team",
      },
    },
  );
});

test("rejects unknown fields and invalid values", () => {
  assert.equal(
    parseProjectChannelRequest({
      type: PROJECT_CHANNEL_REQUEST,
      action: "create",
      requestId: "request-1",
      request: {
        homeChannelId: HOME_CHANNEL,
        name: "release",
        visibility: "public",
      },
    }),
    null,
  );
  assert.equal(
    parseProjectChannelRequest({
      type: PROJECT_CHANNEL_REQUEST,
      action: "create",
      requestId: "request-1",
      request: {
        homeChannelId: HOME_CHANNEL,
        name: "release",
        visibility: "open",
        secret: "nope",
      },
    }),
    null,
  );
});
