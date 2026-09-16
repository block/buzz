import type { EnterpriseSigner } from "./enterprise-signer";
import type { SignedNostrEvent } from "./nostr-signer";

function authorization(event: SignedNostrEvent): string {
  const bytes = new TextEncoder().encode(JSON.stringify(event));
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return `Nostr ${btoa(binary)}`;
}

/** One cache per logged-in signer, never a process-wide or cross-community bearer cache. */
export class EnterpriseMediaCredentials {
  private cached: { header: string; expires: number } | null = null;
  private pending: Promise<string> | null = null;
  private generation = 0;
  private readonly origin: string;
  private readonly signer: Pick<EnterpriseSigner, "signEvent">;
  private readonly now: () => number;

  constructor(
    signer: Pick<EnterpriseSigner, "signEvent">,
    relayHttpUrl: string,
    now: () => number = () => Math.floor(Date.now() / 1000),
  ) {
    this.signer = signer;
    this.now = now;
    const url = new URL(relayHttpUrl);
    if (
      url.protocol !== "https:" ||
      url.username ||
      url.password ||
      url.search ||
      url.hash ||
      url.pathname !== "/"
    )
      throw new Error("Media credentials require an HTTPS relay origin.");
    this.origin = url.origin;
  }

  /** Clear on logout, account/community changes and a relay authentication rejection. */
  clear(): void {
    this.generation++;
    this.cached = null;
    this.pending = null;
  }

  private target(url: string): URL {
    const target = new URL(url);
    if (
      target.origin !== this.origin ||
      target.username ||
      target.password ||
      target.hash
    )
      throw new Error("Media target is outside the enterprise community.");
    return target;
  }

  /** Reuse only before expiry; callers must disable redirects when attaching this bearer header. */
  async read(url: string): Promise<string> {
    this.target(url);
    if (this.cached && this.cached.expires > this.now() + 15)
      return this.cached.header;
    if (this.pending) return this.pending;
    const generation = this.generation;
    const expires = this.now() + 120;
    const pending = this.signer
      .signEvent({
        kind: 24242,
        created_at: this.now(),
        content: "Get buzz-media",
        tags: [
          ["t", "get"],
          ["server", new URL(this.origin).host],
          ["expiration", String(expires)],
        ],
      })
      .then((event) => {
        if (generation !== this.generation)
          throw new Error("Enterprise media identity changed.");
        if (expires <= this.now() + 15)
          throw new Error("Media credential expired while signing.");
        const header = authorization(event);
        this.cached = { header, expires };
        return header;
      })
      .finally(() => {
        if (this.pending === pending) this.pending = null;
      });
    this.pending = pending;
    return pending;
  }

  /** Hash the exact upload bytes; only the hash goes to the signer, never the file. */
  async upload(url: string, bytes: ArrayBuffer): Promise<string> {
    this.target(url);
    const generation = this.generation;
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    const hash = Array.from(new Uint8Array(digest), (b) =>
      b.toString(16).padStart(2, "0"),
    ).join("");
    if (generation !== this.generation)
      throw new Error("Enterprise media identity changed.");
    const expires = this.now() + 120;
    const event = await this.signer.signEvent({
      kind: 24242,
      created_at: this.now(),
      content: "Upload buzz-media",
      tags: [
        ["t", "upload"],
        ["server", new URL(this.origin).host],
        ["expiration", String(expires)],
        ["x", hash],
      ],
    });
    if (generation !== this.generation)
      throw new Error("Enterprise media identity changed.");
    if (expires <= this.now())
      throw new Error("Media credential expired while signing.");
    return authorization(event);
  }
}
