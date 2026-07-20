import * as React from "react";

import {
  type BlobDescriptor,
  appendStagedMediaChunk,
  beginStagedMediaUpload,
  cancelStagedMediaUpload,
  finishStagedMediaUpload,
} from "@/shared/api/tauri";

import { captureVideoPosterFrame } from "./videoPoster";

/**
 * Background upload-jobs store.
 *
 * Media uploads used to live in component-local state inside `useMediaUpload`,
 * so an in-flight upload's spinner vanished the moment you navigated away from
 * its channel, and its completion (the descriptor) was written into unmounted
 * state and lost. This module hoists the whole upload lifecycle into a
 * workspace-scoped singleton:
 *
 *   • The upload **driver** (stage → transcode/upload) runs here, independent of
 *     any React component, so it survives navigation.
 *   • Each job is tagged with the `channelId` it was started in. The composer
 *     renders only its own channel's jobs, so uploads stay attached to the
 *     entry field where they began and never follow you to another channel.
 *   • Finished descriptors land in a per-channel completion buffer; the composer
 *     drains it on mount (or immediately, if already mounted), so a video that
 *     finishes while you're elsewhere is waiting in the right composer when you
 *     return.
 *   • A channel-independent indicator can subscribe to *all* active jobs.
 *
 * Reset on community switch via `resetUploadJobs()` (wired into
 * `resetCommunityState`), matching the other workspace-scoped singletons.
 */

const MEDIA_UPLOAD_CHUNK_BYTES = 1024 * 1024;

/**
 * Store job ids live in a high numeric range so they never collide with the
 * composer's own small, per-mount preview ids. `cancelUpload` routes by range,
 * which lets store-driven previews and local (annotation/paperclip) previews
 * share the numeric `UploadingAttachmentPreview.id` space without a refactor.
 */
export const UPLOAD_JOB_ID_BASE = 1_000_000_000;

export type UploadPhase =
  | "reading"
  | "processing"
  | "uploading"
  | "done"
  | "error";

export type UploadJob = {
  /** Numeric id ≥ UPLOAD_JOB_ID_BASE (see note above). */
  id: number;
  channelId: string;
  filename: string;
  mediaType: string;
  posterUrl?: string;
  /** Monotonic 0–100. */
  pct: number;
  phase: UploadPhase;
  error?: string;
};

type Completion = { jobId: number; descriptor: BlobDescriptor };

// ── Module state ─────────────────────────────────────────────────────────────
let jobs = new Map<number, UploadJob>();
let completedByChannel = new Map<string, Completion[]>();
const canceledIds = new Set<number>();
const uploadIdByJob = new Map<number, string>();
let nextJobId = UPLOAD_JOB_ID_BASE;

// ── Reactivity (mirrors useDrafts' version-counter pattern) ──────────────────
const subscribers = new Set<() => void>();
let version = 0;

function bump(): void {
  version += 1;
  for (const sub of subscribers) sub();
}

export function subscribeUploadJobs(callback: () => void): () => void {
  subscribers.add(callback);
  return () => {
    subscribers.delete(callback);
  };
}

export function getUploadJobsVersion(): number {
  return version;
}

// ── Rust progress events → uploading phase ───────────────────────────────────
let listenerReady = false;
let unlisten: (() => void) | null = null;

/** Correlation id the Rust `media-upload-progress` events carry for a job. */
function progressId(jobId: number): string {
  return `composer-upload-${jobId}`;
}

function parseJobProgressId(id: string): number | null {
  const match = /^composer-upload-(\d+)$/.exec(id);
  if (!match) return null;
  const jobId = Number(match[1]);
  return jobId >= UPLOAD_JOB_ID_BASE ? jobId : null;
}

function ensureProgressListener(): void {
  if (listenerReady) return;
  listenerReady = true;
  void (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const dispose = await listen<{
        id: string;
        phase?: "processing" | "uploading";
        sent: number;
        total: number;
      }>("media-upload-progress", (event) => {
        const { id, phase = "uploading", sent, total } = event.payload;
        if (total <= 0) return;
        const jobId = parseJobProgressId(id);
        if (jobId === null) return;
        const frac = Math.min(1, sent / total);
        if (phase === "processing") {
          // Staging is 0–10%; measured ffmpeg work occupies 10–80%.
          setPhasePct(jobId, "processing", 10 + frac * 70);
        } else {
          // Actual HTTP body bytes occupy 80–99%.
          setPhasePct(jobId, "uploading", 80 + frac * 19);
        }
      });
      if (listenerReady) {
        unlisten = dispose;
      } else {
        dispose();
      }
    } catch {
      // Non-Tauri runtime (web dev, e2e mock) — no byte-level progress.
    }
  })();
}

// ── Percentage model ─────────────────────────────────────────────────────────
// Phases use measurements rather than pretending elapsed wall time is progress:
//   reading/staging → 0–10%   (source bytes handed to Rust)
//   processing      → 10–80%  (ffmpeg out_time / probed duration)
//   uploading       → 80–99%  (actual HTTP bytes-sent / output size)
//   done            → 100%
function setPhasePct(jobId: number, phase: UploadPhase, pct: number): void {
  const job = jobs.get(jobId);
  if (!job) return;
  const next = Math.max(job.pct, Math.min(100, Math.round(pct)));
  if (job.phase === phase && job.pct === next) return;
  job.phase = phase;
  job.pct = next;
  bump();
}

// ── The upload driver ────────────────────────────────────────────────────────
const CANCELED = Symbol("upload-canceled");

async function runUpload(
  jobId: number,
  channelId: string,
  file: File,
): Promise<void> {
  const total = file.size || 0;
  let uploadId: string | undefined;
  try {
    uploadId = await beginStagedMediaUpload();
    if (canceledIds.has(jobId)) throw CANCELED;
    uploadIdByJob.set(jobId, uploadId);

    const reader = file.stream().getReader();
    let staged = 0;
    let pending = new Uint8Array(0);
    for (;;) {
      if (canceledIds.has(jobId)) throw CANCELED;
      const { done, value } = await reader.read();
      if (done) break;
      let bytes = value;
      if (pending.length > 0) {
        const combined = new Uint8Array(pending.length + value.length);
        combined.set(pending);
        combined.set(value, pending.length);
        bytes = combined;
        pending = new Uint8Array(0);
      }
      let offset = 0;
      while (bytes.length - offset >= MEDIA_UPLOAD_CHUNK_BYTES) {
        const chunk = bytes.subarray(offset, offset + MEDIA_UPLOAD_CHUNK_BYTES);
        await appendStagedMediaChunk(uploadId, chunk);
        staged += chunk.length;
        offset += MEDIA_UPLOAD_CHUNK_BYTES;
        if (total > 0) setPhasePct(jobId, "reading", (staged / total) * 10);
      }
      pending = bytes.slice(offset);
    }
    if (pending.length > 0) {
      await appendStagedMediaChunk(uploadId, pending);
      staged += pending.length;
      if (total > 0) setPhasePct(jobId, "reading", (staged / total) * 10);
    }

    if (canceledIds.has(jobId)) throw CANCELED;
    setPhasePct(jobId, "processing", 10);

    const descriptor = await finishStagedMediaUpload(
      uploadId,
      file.name,
      progressId(jobId),
    );
    if (canceledIds.has(jobId)) throw CANCELED;

    const job = jobs.get(jobId);
    if (job) {
      job.phase = "done";
      job.pct = 100;
    }
    const list = completedByChannel.get(channelId) ?? [];
    list.push({ jobId, descriptor });
    completedByChannel.set(channelId, list);
    bump();
  } catch (error) {
    if (uploadId) await cancelStagedMediaUpload(uploadId);
    uploadIdByJob.delete(jobId);
    if (error === CANCELED || canceledIds.has(jobId)) {
      jobs.delete(jobId);
      canceledIds.delete(jobId);
      bump();
      return;
    }
    const job = jobs.get(jobId);
    if (job) {
      job.phase = "error";
      job.error = String(error);
      bump();
    }
  }
}

// ── Public API ───────────────────────────────────────────────────────────────

/** Start a background upload for `file`, attached to `channelId`. Returns the job id. */
export function startChannelUpload(channelId: string, file: File): number {
  ensureProgressListener();
  const id = ++nextJobId;
  jobs.set(id, {
    id,
    channelId,
    filename: file.name,
    mediaType: file.type,
    pct: 0,
    phase: "reading",
  });
  bump();

  if (file.type.startsWith("video/")) {
    void captureVideoPosterFrame(file).then((poster) => {
      const job = jobs.get(id);
      if (poster && job) {
        job.posterUrl = poster.posterUrl;
        bump();
      }
    });
  }

  void runUpload(id, channelId, file);
  return id;
}

/** Cancel a job — stops staging, deletes its temp file, discards any result. */
export function cancelUploadJob(jobId: number): void {
  canceledIds.add(jobId);
  const uploadId = uploadIdByJob.get(jobId);
  if (uploadId) void cancelStagedMediaUpload(uploadId);
  const job = jobs.get(jobId);
  // If already terminal, the driver won't observe the flag — remove directly.
  if (job && (job.phase === "done" || job.phase === "error")) {
    jobs.delete(jobId);
    canceledIds.delete(jobId);
    dropCompletion(job.channelId, jobId);
  }
  bump();
}

function dropCompletion(channelId: string, jobId: number): void {
  const list = completedByChannel.get(channelId);
  if (!list) return;
  const next = list.filter((c) => c.jobId !== jobId);
  if (next.length === 0) completedByChannel.delete(channelId);
  else completedByChannel.set(channelId, next);
}

/** Jobs currently attached to `channelId` (excludes claimed/done). */
export function getChannelUploadJobs(channelId: string): UploadJob[] {
  const out: UploadJob[] = [];
  for (const job of jobs.values()) {
    if (job.channelId === channelId && job.phase !== "done") out.push(job);
  }
  return out;
}

/** All in-flight jobs across every channel (for the global indicator). */
export function getActiveUploadJobs(): UploadJob[] {
  const out: UploadJob[] = [];
  for (const job of jobs.values()) {
    if (
      job.phase === "reading" ||
      job.phase === "processing" ||
      job.phase === "uploading"
    ) {
      out.push(job);
    }
  }
  return out;
}

/** Drain finished descriptors for a channel. Removes the drained jobs. */
export function takeChannelCompletions(channelId: string): BlobDescriptor[] {
  const list = completedByChannel.get(channelId);
  if (!list || list.length === 0) return EMPTY_DESCRIPTORS;
  completedByChannel.delete(channelId);
  for (const { jobId } of list) jobs.delete(jobId);
  bump();
  return list.map((c) => c.descriptor);
}

/** Drain errored jobs for a channel. Removes the drained jobs. */
export function takeChannelErrors(channelId: string): string[] {
  const errors: string[] = [];
  const drained: number[] = [];
  for (const job of jobs.values()) {
    if (job.channelId === channelId && job.phase === "error") {
      errors.push(job.error ?? "Upload failed");
      drained.push(job.id);
    }
  }
  if (drained.length === 0) return EMPTY_ERRORS;
  for (const id of drained) jobs.delete(id);
  bump();
  return errors;
}

const EMPTY_DESCRIPTORS: BlobDescriptor[] = [];
const EMPTY_ERRORS: string[] = [];

/** Tear down on community switch. */
export function resetUploadJobs(): void {
  jobs = new Map();
  completedByChannel = new Map();
  canceledIds.clear();
  uploadIdByJob.clear();
  nextJobId = UPLOAD_JOB_ID_BASE;
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  listenerReady = false;
  bump();
}

// ── React hooks ──────────────────────────────────────────────────────────────

/** Jobs for one channel, re-rendering as they progress. */
export function useChannelUploadJobs(channelId: string | null): UploadJob[] {
  const v = React.useSyncExternalStore(
    subscribeUploadJobs,
    getUploadJobsVersion,
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: `v` (the store version) is the recompute trigger — the selector reads live module state.
  return React.useMemo(
    () => (channelId ? getChannelUploadJobs(channelId) : []),
    [channelId, v],
  );
}

/** All active jobs, for the channel-independent indicator. */
export function useActiveUploadJobs(): UploadJob[] {
  const v = React.useSyncExternalStore(
    subscribeUploadJobs,
    getUploadJobsVersion,
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: `v` (the store version) is the recompute trigger — the selector reads live module state.
  return React.useMemo(() => getActiveUploadJobs(), [v]);
}
