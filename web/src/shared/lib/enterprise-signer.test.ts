import { describe, expect, it, vi, afterEach } from "vitest";
import {
  getEnterpriseSignerSession,
  signWithEnterpriseSigner,
} from "./enterprise-signer";

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.unstubAllEnvs();
});

describe("enterprise signer", () => {
  it("is disabled unless explicitly configured", async () => {
    await expect(getEnterpriseSignerSession()).rejects.toThrow(
      "not configured",
    );
  });

  it("rejects caller-supplied key selectors", async () => {
    vi.stubEnv("VITE_ENTERPRISE_SIGNER_BASE_URL", "https://signer.example");
    await expect(
      signWithEnterpriseSigner(
        {
          kind: 1,
          created_at: 1,
          tags: [],
          content: "hi",
          pubkey: "00",
        } as never,
        "publish",
      ),
    ).rejects.toThrow("must not include pubkey");
  });

  it("requires the signed event to use the server account key", async () => {
    vi.stubEnv("VITE_ENTERPRISE_SIGNER_BASE_URL", "https://signer.example");
    const account = "a".repeat(64);
    const wrong = "b".repeat(64);
    globalThis.fetch = vi.fn(async (_url, init) => {
      const path = String(_url);
      if (path.endsWith("/session")) {
        return new Response(
          JSON.stringify({ pubkeyHex: account, membershipState: "active" }),
          { status: 200 },
        );
      }
      expect(JSON.parse(String(init?.body)).event.pubkey).toBeUndefined();
      return new Response(
        JSON.stringify({
          event: {
            kind: 1,
            created_at: 1,
            tags: [],
            content: "hi",
            id: "id",
            pubkey: wrong,
            sig: "sig",
          },
        }),
        { status: 200 },
      );
    }) as never;

    await expect(
      signWithEnterpriseSigner(
        { kind: 1, created_at: 1, tags: [], content: "hi" },
        "publish",
      ),
    ).rejects.toThrow("wrong account");
  });
});
