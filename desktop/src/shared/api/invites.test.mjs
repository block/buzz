import assert from "node:assert/strict";
import test from "node:test";

import { getJoinPolicy, mintInvite } from "./invites.ts";

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

test("getJoinPolicy maps relay-hosted Markdown and age requirements", async () => {
  await withFetch(
    new Response(
      JSON.stringify({
        policy: {
          terms_markdown: "# Terms",
          privacy_markdown: "# Privacy",
          age_attestation_required: true,
          version: "policy-v1",
        },
      }),
      { status: 200 },
    ),
    async () => {
      assert.deepEqual(await getJoinPolicy("wss://relay.example", "webview"), {
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
    assert.equal(await getJoinPolicy("wss://relay.example", "webview"), null),
  );
  await withFetch(new Response(null, { status: 404 }), async () =>
    assert.equal(await getJoinPolicy("wss://relay.example", "webview"), null),
  );
});

test("getJoinPolicy fails closed on a policy endpoint error", async () => {
  await withFetch(new Response(null, { status: 503 }), async () =>
    assert.rejects(getJoinPolicy("wss://relay.example", "webview"), /HTTP 503/),
  );
});

test("getJoinPolicy maps the native command response", async () => {
  const previousWindow = globalThis.window;
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke(command, args) {
        assert.equal(command, "fetch_join_policy");
        assert.deepEqual(args, { relayUrl: "wss://relay.example" });
        return Promise.resolve({
          terms_markdown: "# Terms",
          privacy_markdown: "# Privacy",
          age_attestation_required: true,
          version: "policy-v1",
        });
      },
    },
  };

  try {
    assert.deepEqual(await getJoinPolicy("wss://relay.example", "native"), {
      termsMarkdown: "# Terms",
      privacyMarkdown: "# Privacy",
      ageAttestationRequired: true,
      version: "policy-v1",
    });
  } finally {
    globalThis.window = previousWindow;
  }
});

// --- mintInvite ---

async function withMintCommand(response, run) {
  const previousWindow = globalThis.window;
  const calls = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke(command, args) {
        calls.push({ command, args });
        return Promise.resolve(response);
      },
    },
  };
  try {
    await run(calls);
  } finally {
    globalThis.window = previousWindow;
  }
}

const MINTED = {
  code: "v2.abc123",
  expires_at: 1785100000,
  url: "https://relay.example/invite/v2.abc123",
  max_uses: 10,
  uses_remaining: 10,
};

test("mintInvite mints through the native command and maps the response", async () => {
  await withMintCommand(MINTED, async (calls) => {
    assert.deepEqual(await mintInvite({ ttlSecs: 259200, maxUses: 10 }), {
      code: "v2.abc123",
      expiresAt: 1785100000,
      url: "https://relay.example/invite/v2.abc123",
      maxUses: 10,
      usesRemaining: 10,
    });
    assert.deepEqual(calls, [
      { command: "mint_invite", args: { ttlSecs: 259200, maxUses: 10 } },
    ]);
  });
});

test("mintInvite sends null max uses for unlimited invites", async () => {
  await withMintCommand(
    { ...MINTED, max_uses: null, uses_remaining: null },
    async (calls) => {
      await mintInvite({ ttlSecs: 86400, maxUses: null });
      await mintInvite({ ttlSecs: 86400 });
      assert.deepEqual(
        calls.map(({ args }) => args),
        [
          { ttlSecs: 86400, maxUses: null },
          { ttlSecs: 86400, maxUses: null },
        ],
      );
    },
  );
});
