import { SELF } from "cloudflare:test";
import { finalizeEvent, generateSecretKey } from "nostr-tools";
import { describe, expect, it } from "vitest";
import {
  hexToBytes,
  TEST_PEER_SECRET_HEX,
  TEST_READER_SECRET_HEX,
  TEST_REPLICATION_SOURCE,
} from "./peer-fixture";

const PEER_SECRET = hexToBytes(TEST_PEER_SECRET_HEX);
const READER_SECRET = hexToBytes(TEST_READER_SECRET_HEX);
const ORIGIN = "https://artifact-gating.example";

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    bytes.buffer as ArrayBuffer,
  );
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

async function nip98Header(
  secretKey: Uint8Array,
  method: "GET" | "POST",
  url: string,
  body: Uint8Array,
): Promise<string> {
  const proof = finalizeEvent(
    {
      kind: 27_235,
      created_at: Math.floor(Date.now() / 1000),
      tags: [
        ["u", url],
        ["method", method],
        ["payload", await sha256Hex(body)],
        ["nonce", crypto.randomUUID()],
      ],
      content: "",
    },
    secretKey,
  );
  return `Nostr ${btoa(JSON.stringify(proof))}`;
}

async function upload(
  secretKey: Uint8Array,
  bytes: Uint8Array,
): Promise<Response> {
  const url = `${ORIGIN}/artifacts`;
  return SELF.fetch(url, {
    method: "POST",
    headers: {
      authorization: await nip98Header(secretKey, "POST", url, bytes),
      "content-type": "application/octet-stream",
    },
    body: bytes,
  });
}

async function fetchArtifact(
  secretKey: Uint8Array,
  sha: string,
  method: "GET" | "HEAD" = "GET",
): Promise<Response> {
  const url = `${ORIGIN}/artifacts/${sha}`;
  return SELF.fetch(url, {
    method,
    headers: {
      authorization: await nip98Header(
        secretKey,
        "GET",
        url,
        new Uint8Array(0),
      ),
    },
  });
}

async function pushReferencingEvent(sha: string): Promise<Response> {
  const event = JSON.parse(
    JSON.stringify(
      finalizeEvent(
        {
          kind: 1,
          created_at: Math.floor(Date.now() / 1000),
          tags: [["x", sha]],
          content: "carries an artifact reference",
        },
        generateSecretKey(),
      ),
    ),
  );
  const url = `${ORIGIN}/replication`;
  const body = JSON.stringify([
    { source: TEST_REPLICATION_SOURCE, cursor: `test-cursor-${sha}`, event },
  ]);
  return SELF.fetch(url, {
    method: "POST",
    headers: {
      authorization: await nip98Header(
        PEER_SECRET,
        "POST",
        url,
        new TextEncoder().encode(body),
      ),
      "content-type": "application/json",
    },
    body,
  });
}

describe("artifact custody follows reference", () => {
  it("admitted peer uploads; granted reader fetches a referenced blob", async () => {
    const bytes = new TextEncoder().encode("rendezvous-custodied artifact");
    const sha = await sha256Hex(bytes);

    const pushed = await pushReferencingEvent(sha);
    expect(pushed.status).toBe(200);

    const stored = await upload(PEER_SECRET, bytes);
    expect(stored.status).toBe(200);
    const receipt = (await stored.json()) as { sha256: string; size: number };
    expect(receipt.sha256).toBe(sha);
    expect(receipt.size).toBe(bytes.length);

    // TEST_READER holds a read grant on the mirror stream, which contains
    // the referencing event — visibility follows reference.
    const fetched = await fetchArtifact(READER_SECRET, sha);
    expect(fetched.status).toBe(200);
    expect(new Uint8Array(await fetched.arrayBuffer())).toEqual(bytes);

    const probed = await fetchArtifact(READER_SECRET, sha, "HEAD");
    expect(probed.status).toBe(200);
  });

  it("denies fetch to a principal without a covering read grant", async () => {
    const bytes = new TextEncoder().encode("granted readers only");
    const sha = await sha256Hex(bytes);
    expect((await pushReferencingEvent(sha)).status).toBe(200);
    expect((await upload(PEER_SECRET, bytes)).status).toBe(200);

    const stranger = generateSecretKey();
    const denied = await fetchArtifact(stranger, sha);
    expect(denied.status).toBe(403);
    expect(((await denied.json()) as { code: string }).code).toBe(
      "scope_denied",
    );
  });

  it("keeps an unreferenced blob invisible even to granted readers", async () => {
    const bytes = new TextEncoder().encode("stored but never referenced");
    const sha = await sha256Hex(bytes);
    expect((await upload(PEER_SECRET, bytes)).status).toBe(200);

    const denied = await fetchArtifact(READER_SECRET, sha);
    expect(denied.status).toBe(403);
  });

  it("denies upload to a principal that is not owner or admitted peer", async () => {
    const denied = await upload(
      generateSecretKey(),
      new TextEncoder().encode("unwelcome bytes"),
    );
    expect(denied.status).toBe(403);
  });

  it("requires authentication on every artifact path", async () => {
    const anonymous = await SELF.fetch(`${ORIGIN}/artifacts/${"a".repeat(64)}`);
    expect(anonymous.status).toBe(401);
  });
});
