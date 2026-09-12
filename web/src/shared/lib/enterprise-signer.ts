import { verifyEvent } from "nostr-tools/pure";
import type { SignedNostrEvent, UnsignedNostrEvent } from "./nostr-signer";

export type EnterpriseSignerSession = {
  pubkey: string;
  relayWsUrl: string;
  relayHttpUrl: string;
};

/** A bearer credential is obtained by the platform login flow, never persisted by this adapter. */
export type EnterpriseSignerOptions = {
  baseUrl: string;
  credential: () => Promise<string>;
  /** Dedicated short-lived corporate API token, not the long-lived app session. */
  corporateAuthorization: () => Promise<string>;
  /** Pin the identity and community established by login; token refresh must not switch either. */
  expectedSession: EnterpriseSignerSession;
};

/** Explicit, HTTPS-only corporate signer. Errors never fall back to another identity. */
export class EnterpriseSigner {
  private readonly baseUrl: string;
  private readonly credential: () => Promise<string>;
  private readonly corporateAuthorization: () => Promise<string>;
  private readonly expectedSession: EnterpriseSignerSession;

  constructor(options: EnterpriseSignerOptions) {
    const url = new URL(options.baseUrl);
    if (
      url.protocol !== "https:" ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    ) {
      throw new Error(
        "Enterprise signer requires an HTTPS URL without credentials, query, or fragment.",
      );
    }
    this.baseUrl = url.toString().replace(/\/+$/, "");
    this.credential = options.credential;
    this.corporateAuthorization = options.corporateAuthorization;
    this.expectedSession = { ...options.expectedSession };
  }

  private async post(path: string, body: unknown): Promise<unknown> {
    const [credential, corporateAuthorization] = await Promise.all([
      this.credential(),
      this.corporateAuthorization(),
    ]);
    if (
      [credential, corporateAuthorization].some(
        (value) => !value || value.length > 16 * 1024 || /[\r\n]/.test(value),
      )
    )
      throw new Error("Corporate login is required.");
    const response = await fetch(
      `${this.baseUrl}/v1/buzz/enterprise-signer/${path}`,
      {
        method: "POST",
        credentials: "omit",
        redirect: "error",
        cache: "no-store",
        headers: {
          "Content-Type": "application/json",
          "X-BB-Session-Credential": credential,
          "X-Buzz-Corporate-Authorization": corporateAuthorization,
        },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(10_000),
      },
    );
    if (!response.ok) {
      await response.body?.cancel();
      throw new Error(`Enterprise signer request failed (${response.status}).`);
    }
    const reader = response.body?.getReader();
    if (!reader) throw new Error("Empty signer response.");
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > 256 * 1024) throw new Error("Signer response is too large.");
        chunks.push(value);
      }
    } finally {
      await reader.cancel();
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return JSON.parse(new TextDecoder().decode(bytes));
  }

  async session(): Promise<EnterpriseSignerSession> {
    const session = (await this.post("session", {})) as EnterpriseSignerSession;
    if (!session || !/^[0-9a-f]{64}$/.test(session.pubkey))
      throw new Error("Invalid enterprise identity.");
    const relay = new URL(session.relayHttpUrl);
    if (
      relay.protocol !== "https:" ||
      relay.username ||
      relay.password ||
      relay.search ||
      relay.hash ||
      relay.pathname !== "/" ||
      session.relayWsUrl !== `wss://${relay.host}`
    )
      throw new Error("Invalid enterprise relay scope.");
    if (
      session.pubkey !== this.expectedSession.pubkey ||
      session.relayWsUrl !== this.expectedSession.relayWsUrl ||
      session.relayHttpUrl !== this.expectedSession.relayHttpUrl
    )
      throw new Error(
        "Enterprise identity or community changed; login is required.",
      );
    return session;
  }

  async signEvent(template: UnsignedNostrEvent): Promise<SignedNostrEvent> {
    if (
      Object.keys(template).sort().join(",") !== "content,created_at,kind,tags"
    ) {
      throw new Error("Signer templates cannot select an identity.");
    }
    // Own a deep snapshot before the first await; a caller must not change what it asked us to sign in flight.
    const event: UnsignedNostrEvent = JSON.parse(JSON.stringify(template));
    const session = await this.session();
    const purpose =
      event.kind === 22242
        ? "nip42-auth"
        : event.kind === 27235
          ? "http-auth"
          : event.kind === 24242
            ? event.tags.some((t) => t[0] === "t" && t[1] === "upload")
              ? "media-upload"
              : "media-read"
            : "publish";
    if (
      purpose === "nip42-auth" &&
      event.tags
        .filter((t) => t[0] === "relay")
        .some((t) => t[1] !== session.relayWsUrl)
    ) {
      throw new Error("Event targets another enterprise relay.");
    }
    const response = (await this.post("events/sign", { event, purpose })) as {
      event: SignedNostrEvent;
    };
    const signed = response?.event;
    if (
      !signed ||
      signed.pubkey !== session.pubkey ||
      signed.kind !== event.kind ||
      signed.created_at !== event.created_at ||
      signed.content !== event.content ||
      JSON.stringify(signed.tags) !== JSON.stringify(event.tags) ||
      !verifyEvent(signed)
    ) {
      throw new Error("Enterprise signer returned an invalid signed event.");
    }
    return signed;
  }
}
