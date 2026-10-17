import type { RelayEvent } from "@/shared/api/types";
import { KIND_HUDDLE_SCREEN } from "@/shared/constants/kinds";

export type ScreenShare = {
  session_id: string;
  started_at: number;
  label: string;
};
export type VideoAccessUnit = { timestamp: number; key: boolean; data: string };
export type VideoBatch = {
  type: "video";
  share: ScreenShare;
  sequence: number;
  codec: string;
  width: number;
  height: number;
  frames: VideoAccessUnit[];
};
export type ScreenMessage =
  | VideoBatch
  | { type: "stopped"; share: ScreenShare };
export const SCREEN_EXPIRY_MS = 5_000;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Parse bounded, room-scoped H.264 batches; JPEG snapshots are not accepted. */
export function parseScreenMessage(
  event: RelayEvent,
  channelId: string,
  now = Date.now(),
): ScreenMessage | null {
  const channels = event.tags.filter((tag) => tag[0] === "h");
  if (
    event.kind !== KIND_HUDDLE_SCREEN ||
    channels.length !== 1 ||
    channels[0].length !== 2 ||
    channels[0][1] !== channelId ||
    event.content.length > 244 * 1024 ||
    Math.abs(event.created_at * 1000 - now) > SCREEN_EXPIRY_MS
  )
    return null;
  try {
    const message = JSON.parse(event.content) as ScreenMessage;
    const share = message.share;
    if (
      !share ||
      typeof share.session_id !== "string" ||
      !UUID.test(share.session_id) ||
      share.session_id === "00000000-0000-0000-0000-000000000000" ||
      !Number.isSafeInteger(share.started_at) ||
      share.started_at <= 0 ||
      share.started_at > now + 5000 ||
      typeof share.label !== "string" ||
      !share.label.trim() ||
      Array.from(share.label).length > 160
    )
      return null;
    share.session_id = share.session_id.toLowerCase();
    if (message.type === "stopped") return message;
    if (
      message.type !== "video" ||
      !Number.isSafeInteger(message.sequence) ||
      message.sequence <= 0 ||
      !/^avc1\.(42|4d|64)[0-9a-f]{4}$/i.test(message.codec) ||
      !Number.isInteger(message.width) ||
      message.width < 2 ||
      message.width > 1920 ||
      !Number.isInteger(message.height) ||
      message.height < 2 ||
      message.height > 1080 ||
      !Array.isArray(message.frames) ||
      message.frames.length === 0 ||
      message.frames.length > 8
    )
      return null;
    let previous = -1;
    let total = 0;
    for (const frame of message.frames) {
      if (
        !Number.isSafeInteger(frame.timestamp) ||
        frame.timestamp < 0 ||
        frame.timestamp <= previous ||
        typeof frame.key !== "boolean" ||
        typeof frame.data !== "string" ||
        !(frame.data.startsWith("AAAAA") || frame.data.startsWith("AAAB")) ||
        !/^[A-Za-z0-9+/]+={0,2}$/.test(frame.data)
      )
        return null;
      previous = frame.timestamp;
      total += frame.data.length;
    }
    if (total > 240_320) return null;
    return message;
  } catch {
    return null;
  }
}

/** Live compressed stream with a bounded one-GOP replay for a newly opened viewer. */
export class ScreenVideoStream {
  readonly pubkey: string;
  readonly share: ScreenShare;
  codec = "";
  width = 0;
  height = 0;
  receivedAt = 0;
  private sequence = 0;
  private timestamp = -1;
  private waitingForKey = true;
  private replay: VideoBatch[] = [];
  private replayBytes = 0;
  private listeners = new Set<(batch: VideoBatch | null) => void>();

  constructor(pubkey: string, share: ScreenShare) {
    this.pubkey = pubkey;
    this.share = share;
  }

  /** Feed timed frames, resetting dependencies after loss or format changes. */
  push(batch: VideoBatch, now: number) {
    if (
      (this.sequence !== 0 && batch.sequence !== this.sequence + 1) ||
      (this.codec &&
        (this.codec !== batch.codec ||
          this.width !== batch.width ||
          this.height !== batch.height))
    ) {
      this.reset();
    }
    this.sequence = batch.sequence;
    this.codec = batch.codec;
    this.width = batch.width;
    this.height = batch.height;
    this.receivedAt = now;
    let frames = batch.frames;
    const keyIndex = frames.findIndex((frame) => frame.key);
    if (this.waitingForKey) {
      if (keyIndex < 0) return;
      frames = frames.slice(keyIndex);
      this.waitingForKey = false;
    }
    if (frames[0].timestamp <= this.timestamp) {
      this.reset();
      return;
    }
    this.timestamp = frames.at(-1)?.timestamp ?? this.timestamp;
    if (keyIndex >= 0) {
      this.replay = [];
      this.replayBytes = 0;
      // The replay begins with the most recent IDR, including one inside a batch.
      const replayFrames = batch.frames.slice(keyIndex);
      this.replay.push({ ...batch, frames: replayFrames });
      this.replayBytes = replayFrames.reduce(
        (size, frame) => size + frame.data.length,
        0,
      );
    } else {
      this.replay.push({ ...batch, frames });
      this.replayBytes += frames.reduce(
        (size, frame) => size + frame.data.length,
        0,
      );
    }
    if (this.replay.length > 12 || this.replayBytes > 2 * 1024 * 1024) {
      this.replay = [];
      this.replayBytes = 0;
    }
    for (const listener of this.listeners) listener({ ...batch, frames });
  }

  /** Attach a player and replay only its current bounded keyframe dependency chain. */
  subscribe(listener: (batch: VideoBatch | null) => void) {
    this.listeners.add(listener);
    for (const batch of this.replay) listener(batch);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private reset() {
    this.waitingForKey = true;
    this.timestamp = -1;
    this.replay = [];
    this.replayBytes = 0;
    for (const listener of this.listeners) listener(null);
  }

  /** Release packets and all decoder subscribers when the share ends. */
  close() {
    this.reset();
    this.listeners.clear();
  }
}

export type SharedScreen = ScreenVideoStream;
type SenderState = {
  session: string;
  startedAt: number;
  sequence: number;
  stopped: boolean;
  touchedAt: number;
};

/** Bounded room receiver; React updates only for stream membership/metadata. */
export class ScreenShareReceiver {
  private senders = new Map<string, SenderState>();
  private screens = new Map<string, ScreenVideoStream>();
  private readonly channelId: string;
  constructor(channelId: string) {
    this.channelId = channelId;
  }

  /** Apply a packet and return whether the visible stream list changed. */
  receive(event: RelayEvent, now = Date.now()): boolean {
    const message = parseScreenMessage(event, this.channelId, now);
    if (!message) return false;
    let changed = this.expire(now);
    const previous = this.senders.get(event.pubkey);
    const sameSession = previous?.session === message.share.session_id;
    if (
      previous &&
      (sameSession
        ? previous.stopped ||
          (message.type === "video" && message.sequence <= previous.sequence)
        : message.share.started_at <= previous.startedAt)
    )
      return changed;
    if (!previous && this.senders.size >= 128) return changed;
    if (
      message.type === "video" &&
      !this.screens.has(event.pubkey) &&
      this.screens.size >= 25
    )
      return changed;
    this.senders.set(event.pubkey, {
      session: message.share.session_id,
      startedAt: message.share.started_at,
      sequence:
        message.type === "video" ? message.sequence : (previous?.sequence ?? 0),
      stopped: message.type === "stopped",
      touchedAt: now,
    });
    let stream = this.screens.get(event.pubkey);
    if (message.type === "stopped") {
      stream?.close();
      return this.screens.delete(event.pubkey) || changed;
    }
    if (!stream || stream.share.session_id !== message.share.session_id) {
      stream?.close();
      stream = new ScreenVideoStream(event.pubkey, message.share);
      this.screens.set(event.pubkey, stream);
      changed = true;
    }
    stream.push(message, Math.min(now, event.created_at * 1000 + 1000));
    return changed;
  }

  /** Expire disconnected streams, including decoder buffers and metadata. */
  expire(now = Date.now()): boolean {
    let changed = false;
    for (const [pubkey, stream] of this.screens) {
      if (now - stream.receivedAt >= SCREEN_EXPIRY_MS) {
        stream.close();
        this.screens.delete(pubkey);
        changed = true;
      }
    }
    for (const [pubkey, sender] of this.senders)
      if (now - sender.touchedAt > 20_000) this.senders.delete(pubkey);
    return changed;
  }

  /** Current stream identities, independent of the 30 FPS rendering cadence. */
  snapshot(): SharedScreen[] {
    return [...this.screens.values()];
  }
  /** Release every stream on disconnect, leave, or community change. */
  clear() {
    for (const stream of this.screens.values()) stream.close();
    this.screens.clear();
    this.senders.clear();
  }
}
