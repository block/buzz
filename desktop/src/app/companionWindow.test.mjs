import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  acceptsNativeDeepLinks,
  agentActivityCompanionCoordinates,
  companionCommunityBootstrap,
  companionCommunityIdForHash,
  companionWindowKindForLabel,
  pinAgentActivityCompanionSearch,
} from "./companionWindow.ts";

describe("companionWindowKindForLabel", () => {
  it("classifies focused companion labels", () => {
    assert.equal(
      companionWindowKindForLabel("agent-activity-deadbeef-channel"),
      "agent-activity",
    );
    assert.equal(companionWindowKindForLabel("huddle-channel-id"), "huddle");
  });

  it("leaves primary and unrelated windows unclassified", () => {
    assert.equal(companionWindowKindForLabel("main"), null);
    assert.equal(companionWindowKindForLabel("reader-document"), null);
  });
});

describe("agentActivityCompanionCoordinates", () => {
  const coordinates = {
    community: "community-one",
    agentSession: "agent-one",
    agentSessionChannel: "channel-one",
  };

  it("returns every immutable activity companion coordinate", () => {
    assert.deepEqual(
      agentActivityCompanionCoordinates("agent-activity", coordinates),
      coordinates,
    );
  });

  it("rejects non-activity windows and incomplete coordinates", () => {
    assert.equal(
      agentActivityCompanionCoordinates("huddle", coordinates),
      undefined,
    );
    assert.equal(
      agentActivityCompanionCoordinates("agent-activity", {
        community: coordinates.community,
        agentSession: coordinates.agentSession,
      }),
      undefined,
    );
  });
});

describe("pinAgentActivityCompanionSearch", () => {
  const coordinates = {
    community: "community-one",
    agentSession: "agent-one",
    agentSessionChannel: "channel-one",
  };

  it("preserves coordinates while stripping panel-swapping state", () => {
    assert.deepEqual(
      pinAgentActivityCompanionSearch("agent-activity", coordinates, {
        community: "community-two",
        agentSession: "agent-two",
        agentSessionChannel: "channel-two",
        messageId: "message-one",
        profile: "profile-one",
        thread: "thread-one",
        unrelated: "kept",
      }),
      { ...coordinates, unrelated: "kept" },
    );
  });

  it("leaves ordinary windows unchanged", () => {
    const nextSearch = { messageId: "message-one" };
    assert.equal(
      pinAgentActivityCompanionSearch(null, coordinates, nextSearch),
      nextSearch,
    );
  });
});

describe("acceptsNativeDeepLinks", () => {
  it("reserves the pending-link queue for the main realm", () => {
    assert.equal(acceptsNativeDeepLinks(null), true);
    assert.equal(acceptsNativeDeepLinks("huddle"), false);
    assert.equal(acceptsNativeDeepLinks("agent-activity"), false);
  });
});

describe("companionCommunityBootstrap", () => {
  it("lets huddle companions boot through normal community selection", () => {
    assert.deepEqual(companionCommunityBootstrap("huddle", ""), {
      initialActiveCommunityId: undefined,
      missingRequiredCommunity: false,
    });
  });

  it("rejects agent activity companions without community context", () => {
    assert.deepEqual(companionCommunityBootstrap("agent-activity", ""), {
      initialActiveCommunityId: undefined,
      missingRequiredCommunity: true,
    });
  });

  it("pins agent activity companions to their encoded community", () => {
    assert.deepEqual(
      companionCommunityBootstrap(
        "agent-activity",
        "#/channels/channel?community=community-one",
      ),
      {
        initialActiveCommunityId: "community-one",
        missingRequiredCommunity: false,
      },
    );
  });
});

describe("companionCommunityIdForHash", () => {
  it("reads and decodes immutable community identity from the route", () => {
    assert.equal(
      companionCommunityIdForHash(
        "#/channels/channel?community=community+%26+one&agentSession=agent",
      ),
      "community & one",
    );
  });

  it("returns null when the bootstrap contract is absent", () => {
    assert.equal(companionCommunityIdForHash("#/channels/channel"), null);
  });
});
