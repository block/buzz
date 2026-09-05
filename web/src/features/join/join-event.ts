/**
 * Join-by-address over the RELAY WIRE — the owner-signed join material
 * (kind 34550), fetched with nothing but the wss:// URL.
 *
 * The relay serves this ONE kind to UNAUTHENTICATED subscribers (see the
 * relay's req handler: the unauthenticated branch is pinned to exactly
 * this kind by a fail-closed shape guard). The event is signed by the
 * community owner's key and parameterized-replaceable by its `d` tag, so
 * the owner re-publishes on invite rotation and every client that knows
 * the address sees the fresh material — no join.json, no second machine.
 *
 * Shape: the content JSON is the same v1 material join.json carries
 * (community.host/name, invite_url, default_channel, rooms) with `name`
 * and `origin` tags alongside. Malformed = null — fail closed, never a
 * guess (the same law as join-material.ts).
 */

import type { JoinMaterial } from "./join-material";

const JOIN_EVENT_KIND = 34550;
const FETCH_TIMEOUT_MS = 8000;

type NostrEvent = {
  id: string;
  pubkey: string;
  kind: number;
  content: string;
  created_at: number;
};

export type JoinEvent = {
  material: JoinMaterial;
  eventId: string;
  ownerPubkey: string;
  createdAt: number;
};

/** Parse the event content JSON into JoinMaterial — same well-formedness
 * rules as join.json; anything malformed is refused whole. */
export function parseJoinEventContent(content: string): JoinMaterial | null {
  let json: unknown;
  try {
    json = JSON.parse(content);
  } catch {
    return null;
  }
  if (json === null || typeof json !== "object") return null;
  const candidate = json as Partial<JoinMaterial> | null;
  const baseOk =
    candidate?.v === 1 &&
    typeof candidate.invite_url === "string" &&
    candidate.invite_url.length > 0 &&
    typeof candidate.default_channel?.id === "string" &&
    candidate.default_channel.id.length > 0 &&
    typeof candidate.community?.host === "string" &&
    candidate.community.host.length > 0;
  if (!baseOk) return null;
  if (candidate.rooms !== undefined) {
    const roomsOk =
      Array.isArray(candidate.rooms) &&
      candidate.rooms.every(
        (r) => typeof r?.id === "string" && typeof r?.name === "string",
      );
    if (!roomsOk) return null;
  }
  return candidate as JoinMaterial;
}

/** Fetch the join material straight off the relay wire, unauthenticated.
 * `null` = none published (or unreachable) — fail closed. */
export async function fetchJoinEvent(wsUrl: string): Promise<JoinEvent | null> {
  let ws: WebSocket;
  try {
    ws = new WebSocket(wsUrl);
  } catch {
    return null;
  }
  return new Promise((resolve) => {
    let settled = false;
    const done = (value: JoinEvent | null) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      try {
        ws.close();
      } catch {
        // already closed
      }
      resolve(value);
    };
    const timer = setTimeout(() => done(null), FETCH_TIMEOUT_MS);
    ws.addEventListener("open", () => {
      ws.send(
        JSON.stringify([
          "REQ",
          "join-material",
          { kinds: [JOIN_EVENT_KIND], limit: 1 },
        ]),
      );
    });
    ws.addEventListener("message", (message) => {
      let frame: unknown;
      try {
        frame = JSON.parse(String(message.data));
      } catch {
        return;
      }
      if (!Array.isArray(frame) || frame.length === 0) return;
      const [type, , payload] = frame as [string, string, unknown];
      if (type === "EVENT") {
        const event = payload as Partial<NostrEvent> | null;
        if (
          event?.kind !== JOIN_EVENT_KIND ||
          typeof event.content !== "string"
        ) {
          return;
        }
        const material = parseJoinEventContent(event.content);
        if (material) {
          done({
            material,
            eventId: String(event.id ?? ""),
            ownerPubkey: String(event.pubkey ?? ""),
            createdAt: Number(event.created_at ?? 0),
          });
        }
        return;
      }
      if (type === "EOSE" || type === "CLOSED") {
        done(null);
      }
    });
    ws.addEventListener("error", () => done(null));
    ws.addEventListener("close", () => done(null));
  });
}
