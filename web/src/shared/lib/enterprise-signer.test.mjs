import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
} from "nostr-tools/pure";
import { EnterpriseSigner } from "./enterprise-signer.ts";
import {
  configureEnterpriseSigner,
  hasDurableSigner,
  signNostrEvent,
} from "./nostr-signer.ts";

const originalFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = originalFetch;
  configureEnterpriseSigner(null);
});
const key = generateSecretKey();
const template = {
  kind: 9,
  created_at: 1000,
  tags: [["h", "channel"]],
  content: "hello\u001b 世界",
};
const session = {
  pubkey: getPublicKey(key),
  relayWsUrl: "wss://buzz.example",
  relayHttpUrl: "https://buzz.example",
};
function signer() {
  return new EnterpriseSigner({
    baseUrl: "https://signer.example/api",
    corporateAuthorization: async () => "test-corporate-token",
    expectedSession: session,
  });
}
function serve(sign = (e) => finalizeEvent(e, key)) {
  globalThis.fetch = async (url, init) => {
    assert.equal(init.redirect, "error");
    assert.equal(init.cache, "no-store");
    assert.equal(init.credentials, "omit");
    assert.equal(init.headers.Authorization, "Bearer test-corporate-token");
    return Response.json(
      String(url).endsWith("/session")
        ? session
        : { event: sign(JSON.parse(init.body).event) },
    );
  };
}

test("production signing seam uses enterprise identity and preserves retry ID", async () => {
  serve();
  configureEnterpriseSigner(signer());
  assert.equal(hasDurableSigner(), true);
  const first = await signNostrEvent(template, { requireNip07: true });
  const retry = await signNostrEvent(template, { requireNip07: true });
  assert.equal(first.pubkey, session.pubkey);
  assert.equal(first.id, retry.id);
});
test("failed corporate authorization never falls back to anonymous signing", async () => {
  globalThis.fetch = async () => new Response(null, { status: 403 });
  configureEnterpriseSigner(signer());
  await assert.rejects(signNostrEvent(template), /403/);
});
test("rejects forged or changed signed events", async () => {
  serve((e) => ({ ...finalizeEvent(e, key), sig: "0".repeat(128) }));
  await assert.rejects(signer().signEvent(template), /invalid signed event/);
  serve((e) => finalizeEvent({ ...e, content: "substituted" }, key));
  await assert.rejects(signer().signEvent(template), /invalid signed event/);
  serve((e) => finalizeEvent(e, generateSecretKey()));
  await assert.rejects(signer().signEvent(template), /invalid signed event/);
});
test("forbids HTTP, embedded credentials and identity selectors before transmitting", async () => {
  let calls = 0;
  globalThis.fetch = async () => {
    calls++;
    throw Error("unexpected");
  };
  for (const baseUrl of [
    "http://signer.example",
    "https://user:pass@signer.example",
    "https://signer.example?token=x",
  ]) {
    assert.throws(
      () =>
        new EnterpriseSigner({
          baseUrl,
          corporateAuthorization: async () => "corporate-token",
          expectedSession: session,
        }),
    );
  }
  await assert.rejects(
    signer().signEvent({ ...template, pubkey: session.pubkey }),
    /identity/,
  );
  assert.equal(calls, 0);
});
test("rejects oversized responses", async () => {
  globalThis.fetch = async () => new Response("x".repeat(256 * 1024 + 1));
  await assert.rejects(signer().session(), /too large/);
});
test("logout fences an in-flight signature", async () => {
  let finish;
  const pending = new Promise((resolve) => {
    finish = resolve;
  });
  configureEnterpriseSigner({ signEvent: () => pending });
  const result = signNostrEvent(template);
  configureEnterpriseSigner(null);
  finish(finalizeEvent(structuredClone(template), key));
  await assert.rejects(result, /identity changed/);
});
test("missing corporate credential does not make a network request", async () => {
  globalThis.fetch = async () => {
    throw Error("should not fetch");
  };
  await assert.rejects(
    new EnterpriseSigner({
      baseUrl: "https://signer.example",
      corporateAuthorization: async () => "",
      expectedSession: session,
    }).session(),
    /login/,
  );
});
test("self custody remains available when enterprise mode is unset", async () => {
  assert.equal(hasDurableSigner(), false);
  assert.ok((await signNostrEvent(template)).sig);
});

test("token refresh cannot silently change pinned identity or community", async () => {
  let signCalls = 0;
  for (const changed of [
    { ...session, pubkey: getPublicKey(generateSecretKey()) },
    {
      ...session,
      relayWsUrl: "wss://other.example",
      relayHttpUrl: "https://other.example",
    },
  ]) {
    globalThis.fetch = async (url) => {
      if (!String(url).endsWith("/session")) signCalls++;
      return Response.json(changed);
    };
    await assert.rejects(
      signer().signEvent(template),
      /identity or community changed/,
    );
  }
  assert.equal(signCalls, 0);
});

test("missing corporate token cannot use an otherwise valid app session", async () => {
  let calls = 0;
  globalThis.fetch = async () => {
    calls++;
    throw Error("unexpected network request");
  };
  const client = new EnterpriseSigner({
    baseUrl: "https://signer.example",
    corporateAuthorization: async () => "",
    expectedSession: session,
  });
  await assert.rejects(client.signEvent(template), /login/);
  assert.equal(calls, 0);
});
