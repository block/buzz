// Desktop-side transcript projection for the bridge: TranscriptItem[] -> redacted, bounded TranscriptEntry[],
// delta tracking (only new/changed entries are sent) and chunk packing (<= MAX_CHUNK_BYTES plaintext).
import {
  MAX_CHUNK_BYTES,
  MAX_ENTRY_TEXT,
  TX_INITIAL_BACKLOG,
  type TranscriptEntry,
  type TranscriptRole,
} from "./protocol";
import { clipText, redactText } from "./redact";

/** Structural subset of the desktop's TranscriptItem (kept loose so this module has no UI imports). */
export type TranscriptItemLike = {
  id: string;
  type: string;
  renderClass?: string;
  role?: string;
  text?: string;
  title?: string;
  outcome?: string;
  timestamp?: string;
  channelId?: string | null;
  toolName?: string;
  status?: string;
  isError?: boolean;
  result?: string;
  // deliberately NOT read: args, sections (raw prompt), descriptor, acpSource
};

const roleOf = (it: TranscriptItemLike): TranscriptRole | null => {
  switch (it.type) {
    case "message":
      return it.role === "user" ? "user" : "assistant";
    case "thought":
      return "thought";
    case "plan":
      return "plan";
    case "tool":
      return "tool";
    case "lifecycle":
      return it.renderClass === "permission"
        ? "permission"
        : it.renderClass === "error"
          ? "error"
          : "status";
    default:
      return null; // metadata (raw prompt sections) and unknown types are never published
  }
};

export function entryFromItem(it: TranscriptItemLike): TranscriptEntry | null {
  const role = roleOf(it);
  if (!role) return null;
  const t = Date.parse(it.timestamp ?? "");
  let raw: string;
  if (role === "tool") {
    // Tool args/results can hold file contents, commands and env: publish name + status + a short redacted result head only.
    raw = it.result ? clipText(it.result, 300) : "";
  } else {
    raw = [it.title && role !== "assistant" && role !== "user" ? it.title : "", it.text ?? "", it.outcome ?? ""]
      .filter(Boolean)
      .join("\n");
  }
  const e: TranscriptEntry = {
    id: it.id,
    t: Number.isFinite(t) ? t : 0,
    role,
    text: clipText(redactText(raw), MAX_ENTRY_TEXT),
  };
  if (role === "tool")
    e.tool = {
      name: clipText(redactText(it.toolName ?? "tool"), 60),
      status: it.status ?? "unknown",
      ...(it.isError ? { error: true } : {}),
    };
  if (it.channelId) e.channelId = it.channelId;
  return e;
}

export function entriesFromItems(items: readonly TranscriptItemLike[]): TranscriptEntry[] {
  const out: TranscriptEntry[] = [];
  for (const it of items) {
    const e = entryFromItem(it);
    if (e) out.push(e);
  }
  return out;
}

const sigOf = (e: TranscriptEntry) =>
  `${e.role}|${e.text.length}|${e.text.slice(-24)}|${e.tool?.status ?? ""}`;

/** Remembers what was already published for one agent so only new/changed entries go out. */
export class TranscriptTracker {
  private sent = new Map<string, string>();
  private first = true;

  /** Returns entries not yet published (or changed since), oldest first. */
  delta(entries: TranscriptEntry[]): TranscriptEntry[] {
    let src = entries;
    if (this.first) src = entries.slice(-TX_INITIAL_BACKLOG);
    const out = src.filter((e) => this.sent.get(e.id) !== sigOf(e));
    return out;
  }

  /** Call after a successful publish of these entries. */
  markSent(entries: TranscriptEntry[]) {
    this.first = false;
    for (const e of entries) this.sent.set(e.id, sigOf(e));
    if (this.sent.size > 800) {
      const keep = [...this.sent].slice(-400);
      this.sent = new Map(keep);
    }
  }
}

/** Splits entries into chunks whose JSON body stays under maxBytes. An oversized single entry is clipped further. */
export function packChunks(
  agentPubkey: string,
  entries: TranscriptEntry[],
  maxBytes = MAX_CHUNK_BYTES,
): TranscriptEntry[][] {
  const enc = new TextEncoder();
  const overhead = enc.encode(JSON.stringify({ agentPubkey, entries: [] })).length + 64;
  const chunks: TranscriptEntry[][] = [];
  let cur: TranscriptEntry[] = [];
  let size = overhead;
  for (let e of entries) {
    let n = enc.encode(JSON.stringify(e)).length + 1;
    if (n + overhead > maxBytes) {
      // pathological (multi-byte heavy) entry: shrink text until it fits
      let t = e.text;
      while (n + overhead > maxBytes && t.length > 16) {
        t = clipText(t, Math.floor(t.length / 2));
        e = { ...e, text: t };
        n = enc.encode(JSON.stringify(e)).length + 1;
      }
    }
    if (cur.length > 0 && size + n > maxBytes) {
      chunks.push(cur);
      cur = [];
      size = overhead;
    }
    cur.push(e);
    size += n;
  }
  if (cur.length) chunks.push(cur);
  return chunks;
}
