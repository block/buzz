/**
 * Voice notes for the phone room — record, hand to the community's
 * speech→text door, post the transcript as the message.
 *
 * The door is DECLARED BY THE COMMUNITY in its join material
 * (`join.json` → `voice.path`); a community that publishes no door gets
 * no mic (fail-closed — the capability is the operator's to offer, never
 * the client's to assume). The door transcribes on the relay's box
 * (whisper.cpp, language pinned), returns the transcript plus the audio's
 * sha256, and deletes the raw audio — so the message that reaches the
 * room is TEXT carrying a digest, never a stored recording.
 *
 * Signing follows the canonical-origin law: the NIP-98 `u` tag names the
 * community's canonical origin while the POST rides the road the stranger
 * was given (same shape as the invite claim).
 */

import type {
  SignedNostrEvent,
  UnsignedNostrEvent,
} from "@/shared/lib/nostr-signer";

/** The lane's tongue order — lv first; the door allowlist agrees. */
export const TONGUE_ORDER = ["lv", "th", "ru", "uk"] as const;

export const TONGUE_LABELS: Record<string, string> = {
  lv: "latviešu",
  th: "ไทย",
  ru: "русский",
  uk: "українська",
};

/** Longest note the door accepts — stop the recorder here, honestly. */
export const MAX_VOICE_SECONDS = 120;

type RoomSigner = (
  unsigned: Omit<UnsignedNostrEvent, "created_at"> & { created_at?: number },
) => Promise<SignedNostrEvent>;

async function sha256Hex(bytes: ArrayBuffer): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

/** The best mime this browser can record voice with; null = it cannot. */
export function pickRecorderMime(): string | null {
  if (typeof MediaRecorder === "undefined") return null;
  for (const mime of [
    "audio/webm;codecs=opus",
    "audio/webm",
    "audio/mp4",
    "audio/ogg;codecs=opus",
  ]) {
    if (MediaRecorder.isTypeSupported(mime)) return mime;
  }
  return null;
}

export type VoiceRecording = {
  /** Resolves the finished note; rejects if recording never started. */
  stop(): Promise<Blob>;
  /** Drop the note without waiting — switching rooms, changing your mind. */
  abandon(): void;
};

export async function startVoiceRecording(): Promise<VoiceRecording> {
  const mime = pickRecorderMime();
  if (!mime) throw new Error("this browser cannot record audio");
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  const recorder = new MediaRecorder(stream, { mimeType: mime });
  const chunks: BlobPart[] = [];
  recorder.ondataavailable = (event) => {
    if (event.data.size > 0) chunks.push(event.data);
  };
  const settled = new Promise<Blob>((resolve, reject) => {
    recorder.onstop = () => {
      stream.getTracks().forEach((track) => {
        track.stop();
      });
      resolve(new Blob(chunks, { type: mime }));
    };
    recorder.onerror = () => {
      stream.getTracks().forEach((track) => {
        track.stop();
      });
      reject(new Error("the microphone stopped working"));
    };
  });
  recorder.start(250);
  return {
    stop: () => {
      if (recorder.state !== "inactive") recorder.stop();
      return settled;
    },
    abandon: () => {
      if (recorder.state !== "inactive") recorder.stop();
      stream.getTracks().forEach((track) => {
        track.stop();
      });
    },
  };
}

export type VoiceTranscript = {
  transcript: string;
  lang: string;
  digest: string;
};

/** POST the note to the community's scribe; plain-language errors only. */
export async function transcribeVoice(options: {
  origin: string;
  canonicalHttp?: string;
  path: string;
  blob: Blob;
  lang: string;
  signer: RoomSigner;
}): Promise<VoiceTranscript> {
  const { origin, canonicalHttp, path, blob, lang, signer } = options;
  const bytes = await blob.arrayBuffer();
  const payload = await sha256Hex(bytes);
  // SIGN the canonical origin; TRANSPORT on the road the user was given.
  const signUrl = `${(canonicalHttp ?? origin).replace(/\/+$/, "")}${path}`;
  const auth = await signer({
    kind: 27235,
    tags: [
      ["u", signUrl],
      ["method", "POST"],
      ["payload", payload],
      ["nonce", crypto.randomUUID()],
    ],
    content: "",
  });
  const response = await fetch(
    `${origin.replace(/\/+$/, "")}${path}?lang=${encodeURIComponent(lang)}`,
    {
      method: "POST",
      headers: {
        Authorization: `Nostr ${btoa(JSON.stringify(auth))}`,
        "Content-Type": blob.type || "audio/webm",
      },
      body: bytes,
      signal: AbortSignal.timeout(300_000),
    },
  );
  const json = (await response.json().catch(() => ({}))) as {
    ok?: boolean;
    transcript?: string;
    lang?: string;
    digest?: string;
    error?: string;
  };
  if (!response.ok || !json.ok || !json.transcript || !json.digest) {
    throw new Error(
      typeof json.error === "string"
        ? json.error
        : `the scribe answered HTTP ${response.status}`,
    );
  }
  return {
    transcript: json.transcript,
    lang: String(json.lang ?? lang),
    digest: String(json.digest),
  };
}

/** The message a voice note becomes: the transcript, plus its anchor. */
export function voiceMessageContent(note: VoiceTranscript): string {
  return `${note.transcript}\n\n— 🎙 voice→text · ${note.lang} · sha256:${note.digest}`;
}
