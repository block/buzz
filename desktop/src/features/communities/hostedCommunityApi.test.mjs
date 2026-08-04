import assert from "node:assert/strict";
import test from "node:test";

import {
  HOSTED_COMMUNITY_SUFFIX,
  hostedCommunityRelayUrl,
} from "./hostedCommunityApi.ts";

test("hostedCommunityRelayUrl: uses normalized_host when present", () => {
  assert.equal(
    hostedCommunityRelayUrl({
      normalized_host: "acme.communities.buzz.xyz",
      name: "other",
    }),
    "wss://acme.communities.buzz.xyz",
  );
});

test("hostedCommunityRelayUrl: strips accidental scheme on host", () => {
  assert.equal(
    hostedCommunityRelayUrl({
      normalized_host: "wss://acme.communities.buzz.xyz",
    }),
    "wss://acme.communities.buzz.xyz",
  );
});

test("hostedCommunityRelayUrl: synthesizes from slug when host missing", () => {
  assert.equal(
    hostedCommunityRelayUrl({ slug: "acme", name: "ignored" }),
    `wss://acme.${HOSTED_COMMUNITY_SUFFIX}`,
  );
});

test("hostedCommunityRelayUrl: synthesizes from name when slug missing", () => {
  assert.equal(
    hostedCommunityRelayUrl({ name: "my-team" }),
    `wss://my-team.${HOSTED_COMMUNITY_SUFFIX}`,
  );
});

test("hostedCommunityRelayUrl: null when no usable label", () => {
  assert.equal(hostedCommunityRelayUrl({}), null);
  assert.equal(hostedCommunityRelayUrl({ name: "Bad Name" }), null);
});
