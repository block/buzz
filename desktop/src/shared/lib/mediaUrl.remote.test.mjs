import assert from "node:assert/strict";
import { test } from "node:test";
import {
  bindNativeIdentity,
  revokeNativeIdentity,
} from "../api/nativeIdentitySession.ts";
import {
  beginRelayOriginFetch,
  mediaProxyUrl,
  rewriteRelayUrl,
} from "./mediaUrl.ts";

test("remote element URLs retain realm generation and fail closed after revocation", () => {
  bindNativeIdentity({
    mode: "remote",
    authState: "authenticated",
    publicIdentity: "a".repeat(64),
    generation: 7,
    workspaceActive: true,
  });
  const hash = "b".repeat(64);
  const proxy = mediaProxyUrl(54321, `${hash}.png?thumb=1`);
  assert.equal(
    proxy,
    `http://127.0.0.1:54321/media/${hash}.png?thumb=1&__buzz_generation=7`,
  );
  beginRelayOriginFetch()("https://relay.example");
  assert.equal(
    rewriteRelayUrl(`https://relay.example/media/${hash}.png`),
    `buzz-media://localhost/media/${hash}.png?__buzz_generation=7`,
  );
  assert.equal(
    rewriteRelayUrl(`https://external.example/media/${hash}.png`),
    `https://external.example/media/${hash}.png`,
  );
  assert.equal(
    new URL(
      mediaProxyUrl(54321, `${hash}.png?__buzz_generation=999`),
    ).searchParams
      .getAll("__buzz_generation")
      .join(","),
    "7",
  );
  assert.throws(
    () => bindNativeIdentity({ mode: "remote", generation: 8 }),
    /fresh renderer/,
  );
  revokeNativeIdentity();
  assert.equal(mediaProxyUrl(54321, `${hash}.png`), "about:blank");
  assert.equal(
    rewriteRelayUrl(`https://relay.example/media/${hash}.png`),
    "about:blank",
  );
  assert.equal(new URL(proxy).searchParams.get("__buzz_generation"), "7");
});
