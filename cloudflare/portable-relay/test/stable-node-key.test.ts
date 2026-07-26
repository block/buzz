import { describe, expect, it } from "vitest";
import {
  StableNodeKeyError,
  stableNodeKeyFromUrl,
} from "../src/stable-node-key";
import cloudflareFixture from "../../../specs/fixtures/portable-relay/cloudflare-v0.1.json";

describe("stableNodeKeyFromUrl", () => {
  it.each(
    cloudflareFixture.routing_expectations.equivalent_urls_share_state,
  )("normalizes equivalent authority %s", (url) => {
    expect(stableNodeKeyFromUrl(url)).toBe(
      cloudflareFixture.stable_nodes.primary.normalized_key,
    );
  });

  it("removes default ports only for their own scheme", () => {
    expect(stableNodeKeyFromUrl("http://coherence.example:80")).toBe(
      "coherence.example",
    );
    for (const example of cloudflareFixture.routing_expectations
      .scheme_aware_examples) {
      expect(stableNodeKeyFromUrl(example.url)).toBe(example.normalized_key);
    }
  });

  it("preserves bracketed IPv6 authorities", () => {
    expect(stableNodeKeyFromUrl("https://[2001:db8::1]:8443")).toBe(
      "[2001:db8::1]:8443",
    );
  });

  it("rejects unsupported schemes and credential-bearing authorities", () => {
    expect(() => stableNodeKeyFromUrl("ftp://coherence.example")).toThrow(
      StableNodeKeyError,
    );
    expect(() =>
      stableNodeKeyFromUrl("https://person@coherence.example"),
    ).toThrow(StableNodeKeyError);
  });
});
