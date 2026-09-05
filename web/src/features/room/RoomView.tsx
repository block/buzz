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
 *
 * VOICE NOTES (voice lane 2026-09-04): when the community declares a
 * speech→text door in its join material, the composer grows a mic. Tap →
 * record; tap again → the note goes to the community's scribe (language
 * pinned, tongue order lv · th · ru · uk) and the TRANSCRIPT is posted as
 * the message, carrying the audio's sha256 — the raw audio is deleted by
 * the door after transcription, so no recording is ever stored. No door
 * declared → no mic rendered (fail-closed).
 */

import type {
  SignedNostrEvent,
  UnsignedNostrEvent,
} from "@/shared/lib/nostr-signer";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import * as React from "react";
import { openRoom, type RoomEvent, type RoomStatus } from "./nostr-room";
import {
  MAX_VOICE_SECONDS,
  TONGUE_LABELS,
  TONGUE_ORDER,
  startVoiceRecording,
  transcribeVoice,
  voiceMessageContent,
  type VoiceRecording,
} from "./voice";

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
  /** The http origin the stranger rode (voice transport rides this road). */
  origin: string;
  /** The community's declared speech→text door; absent = no mic. */
  voice?: { path: string; tongues?: string[] };
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
    origin,
    voice,
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

  // VOICE — the community's tongues in the lane's order (lv · th · ru · uk);
  // first declared tongue is the default, matching the door's own default.
  const voiceTongues = React.useMemo(
    () =>
      voice?.tongues && voice.tongues.length > 0
        ? voice.tongues
        : [...TONGUE_ORDER],
    [voice],
  );
  const [voiceState, setVoiceState] = React.useState<
    "idle" | "recording" | "transcribing"
  >("idle");
  const [voiceSeconds, setVoiceSeconds] = React.useState(0);
  const [tongue, setTongue] = React.useState(voiceTongues[0]);
  const [tongueSheetOpen, setTongueSheetOpen] = React.useState(false);
  const recordingRef = React.useRef<VoiceRecording | null>(null);
  const voiceTimerRef = React.useRef<ReturnType<typeof setInterval> | null>(
    null,
  );

  const stopVoiceTimer = () => {
    if (voiceTimerRef.current !== null) {
      clearInterval(voiceTimerRef.current);
      voiceTimerRef.current = null;
    }
  };

  React.useEffect(() => {
    return () => {
      if (voiceTimerRef.current !== null) clearInterval(voiceTimerRef.current);
    };
  }, []);

  const startRecording = async () => {
    setRefusal(null);
    try {
      const recording = await startVoiceRecording();
      recordingRef.current = recording;
      setVoiceSeconds(0);
      setVoiceState("recording");
      voiceTimerRef.current = setInterval(() => {
        setVoiceSeconds((s) => {
          if (s + 1 >= MAX_VOICE_SECONDS) {
            // the door caps notes at 120s — stop honestly instead of
            // letting the user record something it would refuse
            void finishRecording();
            return s + 1;
          }
          return s + 1;
        });
      }, 1000);
    } catch {
      setRefusal(
        "this browser would not give the room a microphone — no recording was made.",
      );
    }
  };

  const finishRecording = async () => {
    const recording = recordingRef.current;
    if (!recording) return;
    stopVoiceTimer();
    recordingRef.current = null;
    setVoiceState("transcribing");
    try {
      const blob = await recording.stop();
      const note = await transcribeVoice({
        origin,
        canonicalHttp: canonicalRelayUrl
          ?.replace(/^wss:/, "https:")
          .replace(/^ws:/, "http:"),
        path: voice?.path ?? "/voice/",
        blob,
        lang: tongue,
        signer,
      });
      const content = voiceMessageContent(note);
      if (looksLikeSecret(content)) {
        // defense in depth — a transcript is text like any other text
        setRefusal(
          "the transcript looked like it contained a private key (nsec1…), so it was not sent.",
        );
        return;
      }
      setRefusal(null);
      handleRef.current?.send(content);
    } catch (error) {
      setRefusal(
        `the voice note did not become a message — ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    } finally {
      setVoiceState("idle");
      setVoiceSeconds(0);
    }
  };

  // Switching rooms resets the pane FIRST so the previous room's messages
  // never bleed into the next room's read. A recording in flight is
  // abandoned — it belonged to the room it started in.
  React.useEffect(() => {
    // biome-ignore lint/correctness/useExhaustiveDependencies(deps): rerun on room switch by design — the pane resets when the room changes
    setEvents([]);
    setStatus({ state: "connecting" });
    if (recordingRef.current) {
      if (voiceTimerRef.current !== null) {
        clearInterval(voiceTimerRef.current);
        voiceTimerRef.current = null;
      }
      recordingRef.current.abandon();
      recordingRef.current = null;
      setVoiceState("idle");
      setVoiceSeconds(0);
    }
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
    // biome-ignore lint/correctness/useExhaustiveDependencies(deps): scroll to the newest message whenever one arrives
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
            type="button"
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
                type="button"
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
            type="button"
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

      {voiceState !== "idle" ? (
        <p className="border-b border-zinc-200 bg-zinc-50 px-4 py-2 text-xs text-zinc-600">
          {voiceState === "recording"
            ? `recording (${voiceSeconds}s) — tap the mic again to send it to the scribe`
            : "transcribing your voice note — the scribe is writing it down…"}
        </p>
      ) : null}

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
        {voice ? (
          <>
            <button
              type="button"
              aria-label="voice note language"
              className="h-10 rounded-lg border border-zinc-300 bg-white px-2 font-mono text-xs text-zinc-600"
              onClick={() => setTongueSheetOpen(true)}
            >
              {tongue}
            </button>
            <button
              type="button"
              aria-label={
                voiceState === "recording"
                  ? "send the voice note"
                  : "record a voice note"
              }
              className={`h-10 w-12 rounded-lg border text-lg ${
                voiceState === "recording"
                  ? "animate-pulse border-red-500 bg-red-500 text-white"
                  : "border-zinc-300 bg-white text-black"
              }`}
              disabled={voiceState === "transcribing"}
              onClick={() => {
                if (voiceState === "recording") void finishRecording();
                else if (voiceState === "idle") void startRecording();
              }}
            >
              {voiceState === "transcribing" ? "…" : "🎙"}
            </button>
          </>
        ) : null}
        <Button
          className="h-10 bg-black text-white hover:bg-black/90"
          onClick={send}
        >
          send
        </Button>
      </div>

      {voice && tongueSheetOpen ? (
        <div
          className="fixed inset-0 z-40 flex flex-col justify-end bg-black/40"
          role="dialog"
          aria-label="voice note language"
          onMouseDown={(event) => {
            if (event.currentTarget === event.target) setTongueSheetOpen(false);
          }}
        >
          <div className="rounded-t-2xl bg-white px-5 pb-6 pt-4">
            <div className="mx-auto mb-3 h-1 w-10 rounded bg-zinc-300" />
            <h2 className="text-sm font-semibold text-black">
              transcribe in which tongue?
            </h2>
            <p className="mt-2 text-xs leading-relaxed text-zinc-600">
              The scribe transcribes with the language PINNED to your choice —
              it never guesses. The community's order:{" "}
              {voiceTongues.map((t) => TONGUE_LABELS[t] ?? t).join(" · ")}.
            </p>
            <div className="mt-3 space-y-2">
              {voiceTongues.map((code) => (
                <button
                  type="button"
                  key={code}
                  className={`h-10 w-full rounded-lg border text-sm ${
                    code === tongue
                      ? "border-black bg-black font-semibold text-white"
                      : "border-zinc-200 bg-white text-black"
                  }`}
                  onClick={() => {
                    setTongue(code);
                    setTongueSheetOpen(false);
                  }}
                >
                  {TONGUE_LABELS[code] ?? code} · {code}
                </button>
              ))}
            </div>
            <Button
              className="mt-3 h-10 w-full border border-zinc-200 bg-white text-sm text-black"
              onClick={() => setTongueSheetOpen(false)}
            >
              close
            </Button>
          </div>
        </div>
      ) : null}

      {keySheetOpen ? (
        <div
          className="fixed inset-0 z-40 flex flex-col justify-end bg-black/40"
          role="dialog"
          aria-label="your key"
          onMouseDown={(event) => {
            if (event.currentTarget === event.target) setKeySheetOpen(false);
          }}
        >
          <div className="rounded-t-2xl bg-white px-5 pb-6 pt-4">
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
