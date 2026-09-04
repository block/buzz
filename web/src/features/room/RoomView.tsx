/**
 * RoomView — the phone-first landing room for join-by-address.
 *
 * After the claim, this is "in the room": the channel's history arriving
 * live over the authenticated subscription, a composer signed with the same
 * identity that claimed the invite, and the relay's verdicts rendered
 * verbatim whenever it says anything other than yes.
 */

import type {
  SignedNostrEvent,
  UnsignedNostrEvent,
} from "@/shared/lib/nostr-signer";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import * as React from "react";
import { openRoom, type RoomEvent, type RoomStatus } from "./nostr-room";

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
    npub,
    exportSecret,
    signer,
  } = props;

  const [events, setEvents] = React.useState<RoomEvent[]>([]);
  const [status, setStatus] = React.useState<RoomStatus>({
    state: "connecting",
  });
  const [draft, setDraft] = React.useState("");
  const [secretCopied, setSecretCopied] = React.useState(false);
  const listRef = React.useRef<HTMLDivElement>(null);
  const eventsRef = React.useRef<RoomEvent[]>([]);
  eventsRef.current = events;

  const handleRef = React.useRef<{ send: (text: string) => void } | null>(null);

  React.useEffect(() => {
    const roomHandle = openRoom({
      wsUrl,
      canonicalRelayUrl,
      channelId,
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
  }, [wsUrl, canonicalRelayUrl, channelId, signer]);

  React.useEffect(() => {
    if (events.length === 0) return;
    const list = listRef.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [events]);

  const send = () => {
    const text = draft.trim();
    if (!text) return;
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
              #{channelName || "room"}
            </p>
            <p className="truncate text-xs text-zinc-500">
              {communityName || host} · {host}
            </p>
          </div>
          <span className="ml-auto rounded-full bg-zinc-100 px-2 py-0.5 text-[11px] text-zinc-600">
            {status.state === "live"
              ? "live"
              : status.state === "authenticating"
                ? "authenticating…"
                : status.state === "connecting"
                  ? "connecting…"
                  : status.state}
          </span>
        </div>
      </header>

      {status.state === "closed" || status.state === "offline" ? (
        <p className="border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs text-amber-800">
          the relay said: “{status.detail}”
        </p>
      ) : null}

      <div ref={listRef} className="flex-1 space-y-3 overflow-y-auto px-4 py-4">
        {events.length === 0 ? (
          <p className="pt-16 text-center text-sm text-zinc-400">
            {status.state === "live" || status.state === "closed"
              ? "no messages yet — say hello"
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
          placeholder="say hello"
          value={draft}
        />
        <Button
          className="h-10 bg-black text-white hover:bg-black/90"
          onClick={send}
        >
          send
        </Button>
      </div>

      <details className="border-t border-zinc-100 px-4 py-2 text-xs text-zinc-500">
        <summary className="cursor-pointer select-none">
          {npub ? (
            <>
              you are <span className="font-mono">{truncatePubkey(npub)}</span>{" "}
              — a key that lives in this browser. keep it?
            </>
          ) : (
            "you are signing with your NIP-07 extension key"
          )}
        </summary>
        <div className="mt-2 space-y-2 pb-2">
          <p className="leading-relaxed">
            This identity was made on this phone and is stored only in this
            browser. To keep it beyond this browser, copy the secret and keep it
            somewhere safe — anyone holding it is you.
          </p>
          <Button
            className="h-8 bg-zinc-800 text-xs text-white hover:bg-zinc-700"
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
        </div>
      </details>
    </div>
  );
}
