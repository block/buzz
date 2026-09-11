/**
 * RoomView — the phone-first landing room for join-by-address.
 *
 * Two founder-phone findings built in (2026-09-04):
 * · NEVER SEND A SECRET — the composer refuses any message carrying the
 *   bech32 secret prefix (nsec1…) before it leaves the phone, in plain
 *   words; the relay refuses it again server-side (defense in depth). The
 *   "copy the secret" control lives in ITS OWN SHEET, opened from the
 *   header — away from the composer — so the one control that handles the
 *   secret is never next to the control that broadcasts.
 * · ROOM SWITCHER — the community's rooms (operator-curated via join.json)
 *   render as chips under the header; tapping switches the live socket to
 *   that room; the join lands in the default room.
 */

import type {
  SignedNostrEvent,
  UnsignedNostrEvent,
} from "@/shared/lib/nostr-signer";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import * as React from "react";
import { openRoom, type RoomEvent, type RoomStatus } from "./nostr-room";

export type RoomRef = { id: string; name: string };

/**
 * The bech32 secret-key TOKEN shape — the prefix plus at least 15 bech32
 * digits (a real nsec is ~63). Not a bare substring: ordinary words that
 * merely contain "nsec1" inside them must pass. Mirrors the relay-side
 * content_leaks_secret so the two guards agree.
 */
const BECH32 = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

function looksLikeSecret(text: string): boolean {
  let from = 0;
  while (true) {
    const at = text.indexOf("nsec1", from);
    if (at === -1) return false;
    let digits = 0;
    for (let i = at + 5; i < text.length && BECH32.includes(text[i]); i++) {
      digits++;
    }
    if (digits >= 15) return true;
    from = at + 5;
  }
}

function timeLabel(createdAt: number): string {
  const deltaSeconds = Math.max(0, Math.floor(Date.now() / 1000) - createdAt);
  if (deltaSeconds < 60) return "now";
  if (deltaSeconds < 3600) return `${Math.floor(deltaSeconds / 60)}m`;
  if (deltaSeconds < 86400) return `${Math.floor(deltaSeconds / 3600)}h`;
  return `${Math.floor(deltaSeconds / 86400)}d`;
}

export function RoomView(props: {
  communityName: string;
  host: string;
  channelName: string;
  wsUrl: string;
  canonicalRelayUrl?: string;
  channelId: string;
  rooms?: RoomRef[];
  npub: string;
  exportSecret: () => string;
  signer: (
    unsigned: Omit<UnsignedNostrEvent, "created_at"> & { created_at?: number },
  ) => Promise<SignedNostrEvent>;
}) {
  const {
    communityName,
    host,
    channelName,
    wsUrl,
    canonicalRelayUrl,
    channelId,
    rooms,
    npub,
    exportSecret,
    signer,
  } = props;

  const [activeRoomId, setActiveRoomId] = React.useState(channelId);
  const activeRoom = rooms?.find((r) => r.id === activeRoomId);
  const activeRoomName = activeRoom?.name ?? channelName;

  const [events, setEvents] = React.useState<RoomEvent[]>([]);
  const [status, setStatus] = React.useState<RoomStatus>({
    state: "connecting",
  });
  const [draft, setDraft] = React.useState("");
  const [refusal, setRefusal] = React.useState<string | null>(null);
  const [secretCopied, setSecretCopied] = React.useState(false);
  const [keySheetOpen, setKeySheetOpen] = React.useState(false);
  const listRef = React.useRef<HTMLDivElement>(null);

  // Switching rooms resets the pane FIRST so the previous room's messages
  // never bleed into the next room's read.
  React.useEffect(() => {
    setEvents([]);
    setStatus({ state: "connecting" });
  }, [activeRoomId]);

  const handleRef = React.useRef<{ send: (text: string) => void } | null>(null);

  React.useEffect(() => {
    const roomHandle = openRoom({
      wsUrl,
      canonicalRelayUrl,
      channelId: activeRoomId,
      signer,
      onEvent: (event) => {
        setEvents((previous) =>
          [...previous, event].sort((a, b) => a.created_at - b.created_at),
        );
      },
      onStatus: setStatus,
    });
    handleRef.current = roomHandle;
    return () => {
      handleRef.current = null;
      roomHandle.close();
    };
  }, [wsUrl, canonicalRelayUrl, activeRoomId, signer]);

  React.useEffect(() => {
    const list = listRef.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [events]);

  const send = () => {
    const text = draft.trim();
    if (!text) return;
    if (looksLikeSecret(text)) {
      setRefusal(
        "That looks like a private key (nsec1…). Sending it would give it away to the whole room — a message cannot be unsent. Nobody legitimate will ever ask you to post a secret.",
      );
      return;
    }
    setRefusal(null);
    setDraft("");
    handleRef.current?.send(text);
  };

  return (
    <div className="mx-auto flex h-dvh w-full max-w-xl flex-col bg-white">
      <header className="border-b border-zinc-200 px-4 py-3">
        <div className="flex items-center gap-2">
          <span
            className={`h-2.5 w-2.5 rounded-full ${
              status.state === "live"
                ? "bg-emerald-500"
                : status.state === "connecting" ||
                    status.state === "authenticating"
                  ? "bg-amber-400"
                  : "bg-zinc-300"
            }`}
          />
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-black">
              #{activeRoomName || "room"}
            </p>
            <p className="truncate text-xs text-zinc-500">
              {communityName || host} · {host}
            </p>
          </div>
          <button
            className="ml-auto rounded-full bg-zinc-100 px-3 py-1 text-[11px] text-zinc-600"
            onClick={() => setKeySheetOpen(true)}
          >
            key
          </button>
        </div>
        {rooms && rooms.length > 1 ? (
          <nav
            aria-label="rooms"
            className="-mx-1 mt-2 flex gap-1.5 overflow-x-auto pb-1"
          >
            {rooms.map((room) => (
              <button
                key={room.id}
                onClick={() => setActiveRoomId(room.id)}
                className={`whitespace-nowrap rounded-full px-3 py-1.5 text-xs ${
                  room.id === activeRoomId
                    ? "bg-black font-semibold text-white"
                    : "bg-zinc-100 text-zinc-600"
                }`}
              >
                {room.name}
              </button>
            ))}
          </nav>
        ) : null}
      </header>

      {refusal ? (
        <p className="border-b border-amber-300 bg-amber-50 px-4 py-2 text-xs leading-relaxed text-amber-900">
          {refusal}{" "}
          <button
            className="underline"
            onClick={() => {
              setRefusal(null);
              setDraft("");
            }}
          >
            clear the draft
          </button>
        </p>
      ) : null}

      {status.state === "closed" || status.state === "offline" ? (
        <p className="border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs text-amber-800">
          the relay said: “{status.detail}”
        </p>
      ) : null}

      <div ref={listRef} className="flex-1 space-y-3 overflow-y-auto px-4 py-4">
        {events.length === 0 ? (
          <p className="pt-16 text-center text-sm text-zinc-400">
            {status.state === "live" || status.state === "closed"
              ? `no messages here yet — ${activeRoomName} is listening`
              : "connecting to the room…"}
          </p>
        ) : null}
        {events.map((event) => (
          <div key={event.id} className="text-sm">
            <div className="flex items-baseline gap-2">
              <span className="font-mono text-xs font-semibold text-zinc-800">
                {truncatePubkey(event.pubkey)}
              </span>
              <span className="text-[11px] text-zinc-400">
                {timeLabel(event.created_at)}
              </span>
            </div>
            <p className="whitespace-pre-wrap break-words text-zinc-800">
              {event.content}
            </p>
          </div>
        ))}
      </div>

      <div className="flex gap-2 border-t border-zinc-200 px-4 py-3">
        <input
          className="h-10 flex-1 rounded-lg border border-zinc-300 bg-white px-3 text-sm text-black placeholder:text-zinc-400 focus:border-zinc-500 focus:outline-none"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              send();
            }
          }}
          placeholder={`message #${activeRoomName || "room"}`}
          value={draft}
        />
        <Button
          className="h-10 bg-black text-white hover:bg-black/90"
          onClick={send}
        >
          send
        </Button>
      </div>

      {keySheetOpen ? (
        <div
          className="fixed inset-0 z-40 flex flex-col justify-end bg-black/40"
          onClick={() => setKeySheetOpen(false)}
        >
          <div
            className="rounded-t-2xl bg-white px-5 pb-6 pt-4"
            onClick={(event) => event.stopPropagation()}
            role="dialog"
            aria-label="your key"
          >
            <div className="mx-auto mb-3 h-1 w-10 rounded bg-zinc-300" />
            <h2 className="text-sm font-semibold text-black">your key</h2>
            <p className="mt-2 text-xs leading-relaxed text-zinc-600">
              You are{" "}
              <span className="font-mono">
                {npub
                  ? truncatePubkey(npub)
                  : "signing with your NIP-07 extension key"}
              </span>
              . This identity was made on this phone and is stored only in this
              browser. To keep it beyond this browser, copy the secret and keep
              it somewhere safe — anyone holding it is you.{" "}
              <b>Never paste it into a room.</b>
            </p>
            <Button
              className="mt-3 h-10 w-full bg-zinc-900 text-sm text-white hover:bg-zinc-700"
              onClick={() => {
                navigator.clipboard
                  .writeText(exportSecret())
                  .then(() => {
                    setSecretCopied(true);
                    setTimeout(() => setSecretCopied(false), 1500);
                  })
                  .catch(() => {});
              }}
            >
              {secretCopied ? "copied ✓" : "copy the secret (nsec)"}
            </Button>
            <Button
              className="mt-2 h-10 w-full border border-zinc-200 bg-white text-sm text-black"
              onClick={() => setKeySheetOpen(false)}
            >
              close
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
