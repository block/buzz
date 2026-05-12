/**
 * Minimal Nostr client with NIP-01 queries and NIP-42 AUTH.
 *
 * Generates an ephemeral keypair per session to authenticate with the relay.
 * This is sufficient for read-only public queries (e.g. repo listings).
 */

import { generateSecretKey, finalizeEvent } from "nostr-tools/pure";
import { makeAuthEvent } from "nostr-tools/nip42";

export interface NostrFilter {
  ids?: string[];
  authors?: string[];
  kinds?: number[];
  since?: number;
  until?: number;
  limit?: number;
  [tag: `#${string}`]: string[] | undefined;
}

export interface NostrEvent {
  id: string;
  pubkey: string;
  kind: number;
  tags: string[][];
  content: string;
  created_at: number;
  sig: string;
}

const QUERY_TIMEOUT_MS = 10_000;

/** Lazily-generated ephemeral keypair for NIP-42 AUTH. */
let _secretKey: Uint8Array | null = null;
export function getEphemeralKey(): Uint8Array {
  if (!_secretKey) {
    _secretKey = generateSecretKey();
  }
  return _secretKey;
}

/**
 * Open a WebSocket to `wsUrl`, authenticate via NIP-42 if challenged,
 * send a REQ with the given filter, collect EVENTs until EOSE, then
 * close and return them.
 */
export function queryEvents(
  wsUrl: string,
  filter: NostrFilter,
): Promise<NostrEvent[]> {
  return new Promise((resolve, reject) => {
    const events: NostrEvent[] = [];
    const subId = `q-${Date.now().toString(36)}`;
    let settled = false;
    let reqSent = false;

    const ws = new WebSocket(wsUrl);

    const timeout = setTimeout(() => {
      if (!settled) {
        settled = true;
        ws.close();
        reject(new Error(`Relay query timed out after ${QUERY_TIMEOUT_MS}ms`));
      }
    }, QUERY_TIMEOUT_MS);

    const cleanup = () => {
      clearTimeout(timeout);
      try {
        ws.close();
      } catch {
        // ignore
      }
    };

    const sendReq = () => {
      if (!reqSent) {
        reqSent = true;
        ws.send(JSON.stringify(["REQ", subId, filter]));
      }
    };

    ws.addEventListener("open", () => {
      // Wait briefly for an AUTH challenge before sending REQ.
      // Sprout relays always send AUTH, but other relays may not.
      setTimeout(() => sendReq(), 100);
    });

    ws.addEventListener("message", (msg) => {
      let data: unknown;
      try {
        data = JSON.parse(String(msg.data));
      } catch {
        return;
      }
      if (!Array.isArray(data)) return;

      const [type] = data;

      if (type === "AUTH" && typeof data[1] === "string") {
        // NIP-42: relay sent an AUTH challenge — sign and respond.
        const challenge = data[1];
        const sk = getEphemeralKey();
        const template = makeAuthEvent(wsUrl, challenge);
        const signed = finalizeEvent(template, sk);
        ws.send(JSON.stringify(["AUTH", signed]));
        // After AUTH, send our REQ.
        sendReq();
        return;
      }

      if (type === "OK") {
        // AUTH response accepted (or rejected). If we haven't sent REQ yet, do so now.
        sendReq();
        return;
      }

      if (type === "EVENT" && data[1] === subId && data[2]) {
        events.push(data[2] as NostrEvent);
      } else if (type === "EOSE" && data[1] === subId) {
        if (!settled) {
          settled = true;
          cleanup();
          resolve(events);
        }
      } else if (type === "CLOSED" && data[1] === subId) {
        // Subscription was rejected (e.g. auth failed).
        if (!settled) {
          settled = true;
          cleanup();
          const reason =
            typeof data[2] === "string"
              ? data[2]
              : "subscription closed by relay";
          reject(new Error(reason));
        }
      } else if (type === "NOTICE") {
        // Informational notice from relay — ignore for now.
      }
    });

    ws.addEventListener("error", () => {
      if (!settled) {
        settled = true;
        cleanup();
        reject(new Error("WebSocket connection failed"));
      }
    });

    ws.addEventListener("close", () => {
      if (!settled) {
        settled = true;
        clearTimeout(timeout);
        resolve(events);
      }
    });
  });
}
