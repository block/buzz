import assert from "node:assert/strict";
import test from "node:test";

import {
  acceptJoinPolicy,
  claimInvite,
  getJoinPolicy,
  mintInvite,
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

test("acceptJoinPolicy binds the exact code, version, and age confirmation", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    assert.equal(url, "https://relay.example/api/invites/accept-policy");
    assert.deepEqual(JSON.parse(init.body), {
      code: "invite-code",
      policy_version: "policy-v1",
      age_confirmed: true,
    });
    return new Response(JSON.stringify({ receipt: "fresh-receipt" }));
  };
  try {
    assert.equal(
      await acceptJoinPolicy(
        "wss://relay.example",
        "invite-code",
        "policy-v1",
        true,
      ),
      "fresh-receipt",
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("acceptJoinPolicy rejects a malformed receipt before claim", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => new Response(JSON.stringify({}));
  try {
    await assert.rejects(
      acceptJoinPolicy(
        "wss://relay.example",
        "invite-code",
        "policy-v1",
        false,
      ),
      /invalid_policy_receipt/,
    );
  } finally {
    globalThis.fetch = originalFetch;
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

test("mintInvite omits max_uses when null (unlimited)", async () => {
  setupTauriStubs("https://relay.example");
  try {
    const originalFetch = globalThis.fetch;
    let capturedBody;
    globalThis.fetch = async (_url, init) => {
      capturedBody = JSON.parse(init.body);
      return new Response(
        JSON.stringify({
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

test("claimInvite marks exact v3 payloads and signs every attempt with a fresh nonce", async () => {
  const tauri = setupTauriStubs("https://relay.example");
  const originalFetch = globalThis.fetch;
  const requests = [];
  globalThis.fetch = async (_url, init) => {
    requests.push({
      body: JSON.parse(init.body),
      authorization: init.headers.Authorization,
    });
    return new Response(
      JSON.stringify({
        status: "joined",
        community_id: "community-id",
        host: "relay.example",
        role: "member",
      }),
    );
  };

  try {
    const code = `v3.${"a".repeat(64)}`;
    await claimInvite("wss://relay.example", code, "policy-receipt");
    await claimInvite("wss://relay.example", code, "policy-receipt");

    assert.equal(requests[0].body.protocol, "identity-handoff-v3");
    assert.equal(requests[0].body.code, code);
    assert.equal(requests[0].body.policy_receipt, "policy-receipt");

    const nonces = tauri.invokeArgs
      .filter(({ command }) => command === "sign_event")
      .map(({ args }) => args.tags.find(([name]) => name === "nonce")?.[1]);
    assert.match(nonces[0], /^[0-9a-f-]{36}$/);
    assert.notEqual(nonces[0], nonces[1]);
  } finally {
    globalThis.fetch = originalFetch;
    teardownTauriStubs();
  }
});

test("claimInvite keeps generic payload serialization byte-compatible", async () => {
  setupTauriStubs("https://relay.example");
  const originalFetch = globalThis.fetch;
  let body;
  globalThis.fetch = async (_url, init) => {
    body = init.body;
    return new Response(
      JSON.stringify({
        status: "already_member",
        community_id: "community-id",
        host: "relay.example",
        role: "member",
      }),
    );
  };

  try {
    await claimInvite("wss://relay.example", "generic-code", "receipt");
    assert.equal(
      body,
      JSON.stringify({ code: "generic-code", policy_receipt: "receipt" }),
    );
  } finally {
    globalThis.fetch = originalFetch;
    teardownTauriStubs();
  }
});
