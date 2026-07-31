import assert from "node:assert/strict";
import test from "node:test";

import {
  getJoinPolicy,
  listActiveGuestInvites,
  mintInvite,
  revokeInvite,
} from "./invites.ts";

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

// --- mintInvite serialization ---

// The test-loader transpiles TS imports. tauri.ts imports `invoke` from
// @tauri-apps/api/core, which calls `window.__TAURI_INTERNALS__.invoke`.
// We stub that here so getRelayHttpUrl() and signRelayEvent() work in node.

function setupTauriStubs(
  httpBase,
  authEvent = {
    id: "x",
    sig: "y",
    pubkey: "z",
    kind: 27235,
    created_at: 1,
    tags: [],
  },
) {
  const calls = { invokeArgs: [] };
  globalThis.window = globalThis.window ?? {};
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      calls.invokeArgs.push({ command, args });
      if (command === "get_relay_http_url") return httpBase;
      if (command === "sign_event") return JSON.stringify(authEvent);
      throw new Error(`Unexpected Tauri command: ${command}`);
    },
  };
  return calls;
}

function teardownTauriStubs() {
  delete globalThis.window.__TAURI_INTERNALS__;
}

test("mintInvite serializes bounded max_uses in the request body", async () => {
  setupTauriStubs("https://relay.example");
  try {
    const originalFetch = globalThis.fetch;
    let capturedBody;
    globalThis.fetch = async (_url, init) => {
      capturedBody = JSON.parse(init.body);
      return new Response(
        JSON.stringify({
          invite_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
          code: "v2.abc123",
          expires_at: 1785100000,
          url: "https://relay.example/invite/v2.abc123",
          max_uses: 10,
          uses_remaining: 10,
        }),
      );
    };
    try {
      const result = await mintInvite({ ttlSecs: 259200, maxUses: 10 });
      assert.equal(capturedBody.ttl_secs, 259200);
      assert.equal(capturedBody.max_uses, 10);
      assert.equal(result.code, "v2.abc123");
      assert.equal(result.inviteId, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
      assert.equal(result.maxUses, 10);
      assert.equal(result.usesRemaining, 10);
      assert.equal(result.expiresAt, 1785100000);
      assert.equal(result.url, "https://relay.example/invite/v2.abc123");
    } finally {
      globalThis.fetch = originalFetch;
    }
  } finally {
    teardownTauriStubs();
  }
});

test("mintInvite binds guest links to the requested channel", async () => {
  setupTauriStubs("https://relay.example");
  try {
    const originalFetch = globalThis.fetch;
    let capturedBody;
    globalThis.fetch = async (_url, init) => {
      capturedBody = JSON.parse(init.body);
      return new Response(
        JSON.stringify({
          invite_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
          channel_id: "11111111-1111-4111-8111-111111111111",
          code: "v2.guest",
          expires_at: 1785100000,
          max_uses: 1,
          role: "guest",
          url: "https://relay.example/invite/v2.guest",
          uses_remaining: 1,
        }),
      );
    };
    try {
      const result = await mintInvite({
        channelId: "11111111-1111-4111-8111-111111111111",
        ttlSecs: 259200,
      });
      assert.equal(
        capturedBody.channel_id,
        "11111111-1111-4111-8111-111111111111",
      );
      assert.equal(capturedBody.max_uses, 1);
      assert.equal(result.role, "guest");
      assert.equal(result.channelId, capturedBody.channel_id);
    } finally {
      globalThis.fetch = originalFetch;
    }
  } finally {
    teardownTauriStubs();
  }
});

test("mintInvite omits max_uses when null (unlimited)", async () => {
  setupTauriStubs("https://relay.example");
  try {
    const originalFetch = globalThis.fetch;
    let capturedBody;
    globalThis.fetch = async (_url, init) => {
      capturedBody = JSON.parse(init.body);
      return new Response(
        JSON.stringify({
          invite_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
          code: "v2.abc123",
          expires_at: 1785100000,
          url: "https://relay.example/invite/v2.abc123",
          max_uses: null,
          uses_remaining: null,
        }),
      );
    };
    try {
      const result = await mintInvite({ ttlSecs: 259200, maxUses: null });
      assert.equal(capturedBody.ttl_secs, 259200);
      assert.equal(Object.hasOwn(capturedBody, "max_uses"), false);
      assert.equal(result.maxUses, null);
      assert.equal(result.usesRemaining, null);
    } finally {
      globalThis.fetch = originalFetch;
    }
  } finally {
    teardownTauriStubs();
  }
});

test("mintInvite omits max_uses when not provided (unlimited default)", async () => {
  setupTauriStubs("https://relay.example");
  try {
    const originalFetch = globalThis.fetch;
    let capturedBody;
    globalThis.fetch = async (_url, init) => {
      capturedBody = JSON.parse(init.body);
      return new Response(
        JSON.stringify({
          invite_id: "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
          code: "v2.abc123",
          expires_at: 1785100000,
          url: "https://relay.example/invite/v2.abc123",
          max_uses: null,
          uses_remaining: null,
        }),
      );
    };
    try {
      await mintInvite({ ttlSecs: 86400 });
      assert.equal(capturedBody.ttl_secs, 86400);
      assert.equal(Object.hasOwn(capturedBody, "max_uses"), false);
    } finally {
      globalThis.fetch = originalFetch;
    }
  } finally {
    teardownTauriStubs();
  }
});

test("listActiveGuestInvites maps active guest-link metadata", async () => {
  setupTauriStubs("https://relay.example");
  try {
    const originalFetch = globalThis.fetch;
    let capturedUrl;
    let capturedBody;
    globalThis.fetch = async (url, init) => {
      capturedUrl = url;
      capturedBody = JSON.parse(init.body);
      return new Response(
        JSON.stringify({
          invites: [
            {
              invite_id: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee",
              expires_at: 1785100000,
              created_at: 1785000000,
            },
          ],
        }),
      );
    };
    try {
      assert.deepEqual(
        await listActiveGuestInvites("11111111-1111-4111-8111-111111111111"),
        [
          {
            inviteId: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee",
            expiresAt: 1785100000,
            createdAt: 1785000000,
          },
        ],
      );
      assert.equal(capturedUrl, "https://relay.example/api/invites/list");
      assert.deepEqual(capturedBody, {
        channel_id: "11111111-1111-4111-8111-111111111111",
      });
    } finally {
      globalThis.fetch = originalFetch;
    }
  } finally {
    teardownTauriStubs();
  }
});

test("revokeInvite sends the mint-time invite ID", async () => {
  setupTauriStubs("https://relay.example");
  try {
    const originalFetch = globalThis.fetch;
    let capturedUrl;
    let capturedBody;
    globalThis.fetch = async (url, init) => {
      capturedUrl = url;
      capturedBody = JSON.parse(init.body);
      return new Response(JSON.stringify({ status: "revoked" }));
    };
    try {
      await revokeInvite("ffffffff-ffff-4fff-8fff-ffffffffffff");
      assert.equal(capturedUrl, "https://relay.example/api/invites/revoke");
      assert.deepEqual(capturedBody, {
        invite_id: "ffffffff-ffff-4fff-8fff-ffffffffffff",
      });
    } finally {
      globalThis.fetch = originalFetch;
    }
  } finally {
    teardownTauriStubs();
  }
});
