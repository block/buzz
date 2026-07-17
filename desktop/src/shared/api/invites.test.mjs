import assert from "node:assert/strict";
import test from "node:test";

import { getJoinPolicy, joinPolicyDocumentUrl } from "./invites.ts";

function withFetch(response, run) {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    assert.equal(url, "https://relay.example/api/join-policy");
    return response;
  };
  return Promise.resolve(run()).finally(() => {
    globalThis.fetch = originalFetch;
  });
}

test("joinPolicyDocumentUrl creates independent document URLs", () => {
  assert.equal(
    joinPolicyDocumentUrl("wss://relay.example", "content-guidelines"),
    "https://relay.example/api/join-policy/content-guidelines",
  );
  assert.equal(
    joinPolicyDocumentUrl("wss://relay.example", "terms"),
    "https://relay.example/api/join-policy/terms",
  );
  assert.equal(
    joinPolicyDocumentUrl("wss://relay.example", "privacy"),
    "https://relay.example/api/join-policy/privacy",
  );
});

test("getJoinPolicy maps relay-hosted Markdown and age requirements", async () => {
  await withFetch(
    new Response(
      JSON.stringify({
        policy: {
          content_guidelines_markdown: "# Content Guidelines",
          terms_markdown: "# Terms",
          privacy_markdown: "# Privacy",
          age_attestation_required: true,
          version: "policy-v1",
        },
      }),
      { status: 200 },
    ),
    async () => {
      assert.deepEqual(await getJoinPolicy("wss://relay.example"), {
        contentGuidelinesMarkdown: "# Content Guidelines",
        termsMarkdown: "# Terms",
        privacyMarkdown: "# Privacy",
        ageAttestationRequired: true,
        version: "policy-v1",
      });
    },
  );
});

test("getJoinPolicy preserves opt-in behavior for unconfigured and older relays", async () => {
  await withFetch(new Response(JSON.stringify({}), { status: 200 }), async () =>
    assert.equal(await getJoinPolicy("wss://relay.example"), null),
  );
  await withFetch(new Response(null, { status: 404 }), async () =>
    assert.equal(await getJoinPolicy("wss://relay.example"), null),
  );
});

test("getJoinPolicy fails closed on a policy endpoint error", async () => {
  await withFetch(new Response(null, { status: 503 }), async () =>
    assert.rejects(getJoinPolicy("wss://relay.example"), /HTTP 503/),
  );
});
