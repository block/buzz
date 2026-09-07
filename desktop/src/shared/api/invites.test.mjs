import assert from "node:assert/strict";
import test from "node:test";

import { claimInvite, getJoinPolicy, mintInvite } from "./invites.ts";

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
    globalThis.fetch = async (url, init) => {
      if (String(url).endsWith("/info")) {
        return new Response(JSON.stringify({}), { status: 200 });
      }
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
    globalThis.fetch = async (url, init) => {
      if (String(url).endsWith("/info")) {
        return new Response(JSON.stringify({}), { status: 200 });
      }
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
    globalThis.fetch = async (url, init) => {
      if (String(url).endsWith("/info")) {
        return new Response(JSON.stringify({}), { status: 200 });
      }
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

// --- canonical-origin signing (alias-host auth fix) ---

function setupClaimFetch(infoBody, claimResult = { status: "joined" }) {
  const calls = { urls: [], claimInit: null };
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.urls.push(String(url));
    if (String(url).endsWith("/info")) {
      return new Response(JSON.stringify(infoBody), { status: 200 });
    }
    calls.claimInit = init;
    return new Response(
      JSON.stringify({
        status: claimResult.status,
        community_id: "cid",
        host: "beehivenature.buzz",
        role: "member",
      }),
    );
  };
  return {
    calls,
    restore() {
      globalThis.fetch = originalFetch;
    },
  };
}

function signedUTag(calls) {
  const signCall = calls.invokeArgs.find((c) => c.command === "sign_event");
  assert.ok(signCall, "sign_event was invoked");
  return signCall.args.tags.find((t) => t[0] === "u")?.[1];
}

test("claimInvite signs the canonical origin from /info while requesting the transport host", async () => {
  // relay2.skaists.dev advertises beehivenature.buzz — the alias-host case
  const tauri = setupTauriStubs("https://unused-active-relay.example");
  const fetchMock = setupClaimFetch({
    push: { origin: "wss://beehivenature.buzz" },
  });
  try {
    const result = await claimInvite("wss://relay2.skaists.dev", "v2.code");
    assert.equal(result.status, "joined");
    assert.equal(result.host, "beehivenature.buzz");
    // the road: /info and the claim POST both rode the transport host
    assert.deepEqual(fetchMock.calls.urls, [
      "https://relay2.skaists.dev/info",
      "https://relay2.skaists.dev/api/invites/claim",
    ]);
    // the identity: the u tag is signed against the CANONICAL origin
    assert.equal(
      signedUTag(tauri),
      "https://beehivenature.buzz/api/invites/claim",
    );
  } finally {
    fetchMock.restore();
    teardownTauriStubs();
  }
});

test("claimInvite keeps signing the transport host when /info advertises no canonical origin", async () => {
  // plain relay: no push.origin — the road IS the identity
  const tauri = setupTauriStubs("https://unused-active-relay.example");
  const fetchMock = setupClaimFetch({ name: "Buzz Relay" });
  try {
    await claimInvite("wss://relay.example", "v2.code");
    assert.equal(
      signedUTag(tauri),
      "https://relay.example/api/invites/claim",
    );
  } finally {
    fetchMock.restore();
    teardownTauriStubs();
  }
});

test("claimInvite fails closed on a malformed canonical advertisement", async () => {
  for (const bad of [
    { push: { origin: "not a url" } },
    { push: { origin: "ftp://beehivenature.buzz" } },
    { push: { origin: "wss://localhost9" } },
    { push: { origin: 42 } },
  ]) {
    const tauri = setupTauriStubs("https://unused-active-relay.example");
    const fetchMock = setupClaimFetch(bad);
    try {
      await assert.rejects(
        claimInvite("wss://relay2.skaists.dev", "v2.code"),
        /canonical origin|URL verification|malformed/,
      );
      // nothing was claimed — no POST left the client
      assert.equal(
        fetchMock.calls.urls.filter((u) => u.endsWith("/claim")).length,
        0,
      );
    } finally {
      fetchMock.restore();
      teardownTauriStubs();
    }
  }
});
