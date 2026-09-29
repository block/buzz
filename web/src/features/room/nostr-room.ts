/**
 * Persistent NIP-42 room connection — the live wire under the room view.
 *
 * One WebSocket to the relay the user was given (never a third party):
 *   AUTH on challenge (NIP-42) → REQ history + live subscription for the
 *   channel (NIP-29 shape: kind 9, `#h` = channel id) → send kind 9.
 *
 * Every relay verdict surfaces verbatim: CLOSED reasons, NOTICEs and send
 * rejections are passed to the UI, never swallowed — a members-only relay
 * refusing an unknown key is a fact the stranger is owed, not an error to
 * hide. Fail-closed is the relay's law; this client just refuses to pretend.
 */

import type {
  SignedNostrEvent,
  UnsignedNostrEvent,
} from "@/shared/lib/nostr-signer";

export type RoomEvent = {
  id: string;
  pubkey: string;
  created_at: number;
  content: string;
};

export type RoomStatus =
  | { state: "connecting" }
  | { state: "authenticating" }
  | { state: "live" }
  | { state: "closed"; detail: string }
  | { state: "offline"; detail: string };

export type RoomSigner = (
  unsigned: Omit<UnsignedNostrEvent, "created_at"> & { created_at?: number },
) => Promise<SignedNostrEvent>;

export type RoomHandle = {
  send(text: string): void;
  close(): void;
};

const KIND_CHANNEL_MESSAGE = 9;
const KIND_AUTH = 22242;

export function openRoom(options: {
  wsUrl: string;
  /** The community's canonical WS origin for the NIP-42 `relay` tag. */
  canonicalRelayUrl?: string;
  channelId: string;
  limit?: number;
  signer: RoomSigner;
  onEvent: (event: RoomEvent) => void;
  onStatus: (status: RoomStatus) => void;
}): RoomHandle {
  const { wsUrl, channelId, signer, onEvent, onStatus } = options;
  const authRelayTag = options.canonicalRelayUrl || wsUrl;
  const limit = options.limit ?? 50;
  const subId = `room-${crypto.randomUUID().slice(0, 8)}`;
  const seen = new Set<string>();
  let closed = false;
  let authEventId: string | null = null;
  let ws: WebSocket;

  try {
    ws = new WebSocket(wsUrl);
  } catch (error) {
    onStatus({ state: "offline", detail: String(error) });
    return { send: () => {}, close: () => {} };
  }

  onStatus({ state: "connecting" });

  const pushEvent = (event: RoomEvent) => {
    if (seen.has(event.id)) return;
    seen.add(event.id);
    onEvent(event);
  };

  const sendReq = () => {
    if (closed || ws.readyState !== WebSocket.OPEN) return;
    if (!sendReq.done) {
      sendReq.done = true;
      ws.send(
        JSON.stringify([
          "REQ",
          subId,
          { kinds: [KIND_CHANNEL_MESSAGE], "#h": [channelId], limit },
        ]),
      );
    }
  };
  sendReq.done = false;

  // A relay that never challenges should still get its REQ; one that does
  // challenges first, and the REQ follows its OK for the AUTH event — asking
  // early just earns "auth-required".
  let reqTimer: ReturnType<typeof setTimeout> | null = setTimeout(() => {
    if (authEventId === null) sendReq();
  }, 1500);

  ws.addEventListener("open", () => {
    if (closed) return;
    if (reqTimer === null) sendReq();
  });

  ws.addEventListener("message", (message) => {
    if (closed) return;
    let parsed: unknown;
    try {
      parsed = JSON.parse(String(message.data));
    } catch {
      return;
    }
    if (!Array.isArray(parsed) || parsed.length === 0) return;
    const [type, first, second, third] = parsed as [
      string,
      unknown,
      unknown,
      unknown,
    ];

    if (type === "AUTH" && typeof first === "string") {
      onStatus({ state: "authenticating" });
      signer({
        kind: KIND_AUTH,
        tags: [
          ["relay", authRelayTag],
          ["challenge", first],
        ],
        content: "",
      })
        .then((signed) => {
          if (closed) return;
          authEventId = signed.id;
          ws.send(JSON.stringify(["AUTH", signed]));
        })
        .catch((error: unknown) => {
          onStatus({
            state: "closed",
            detail: `signing the auth challenge failed: ${String(error)}`,
          });
        });
      return;
    }

    // The AUTH event's own OK: true → subscribe now; false → the relay's
    // verdict, verbatim.
    if (type === "OK" && first === authEventId) {
      if (second === true) {
        if (reqTimer) clearTimeout(reqTimer);
        reqTimer = null;
        sendReq();
      } else {
        onStatus({
          state: "closed",
          detail: typeof third === "string" ? third : "authentication refused",
        });
      }
      return;
    }

    if (
      type === "EVENT" &&
      first === subId &&
      second &&
      typeof second === "object"
    ) {
      const raw = second as {
        id?: unknown;
        pubkey?: unknown;
        created_at?: unknown;
        content?: unknown;
      };
      if (
        typeof raw.id === "string" &&
        typeof raw.pubkey === "string" &&
        typeof raw.created_at === "number" &&
        typeof raw.content === "string"
      ) {
        pushEvent({
          id: raw.id,
          pubkey: raw.pubkey,
          created_at: raw.created_at,
          content: raw.content,
        });
      }
      return;
    }

    if (type === "EOSE" && first === subId) {
      onStatus({ state: "live" });
      return;
    }

    if (type === "NOTICE" && typeof first === "string") {
      onStatus({ state: "closed", detail: first });
      return;
    }

    if (type === "CLOSED" && first === subId && typeof second === "string") {
      onStatus({ state: "closed", detail: second });
      return;
    }

    if (type === "OK" && typeof first === "string" && second === false) {
      onStatus({
        state: "closed",
        detail:
          typeof third === "string" ? third : "the relay refused the event",
      });
      return;
    }
  });

  ws.addEventListener("error", () => {
    if (!closed)
      onStatus({ state: "offline", detail: "the connection failed" });
  });

  ws.addEventListener("close", () => {
    if (!closed)
      onStatus({ state: "offline", detail: "the connection closed" });
  });

  return {
    send(text: string) {
      if (closed || ws.readyState !== WebSocket.OPEN) return;
      signer({
        kind: KIND_CHANNEL_MESSAGE,
        tags: [["h", channelId]],
        content: text,
      })
        .then((signed) => {
          if (!closed && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify(["EVENT", signed]));
          }
        })
        .catch((error: unknown) => {
          onStatus({
            state: "closed",
            detail: `signing failed: ${String(error)}`,
          });
        });
    },
    close() {
      closed = true;
      if (reqTimer) clearTimeout(reqTimer);
      try {
        ws.close();
      } catch {
        // already gone
      }
    },
  };
}
