import * as React from "react";

import { uploadMediaBytes } from "@/shared/api/tauri";
import { cancelMediaUpload, releaseMediaUpload } from "@/shared/api/tauriMedia";
import type { SupportedLinkPreview } from "@/shared/lib/linkPreview";
import {
  buildLinkPreviewSnapshotTag,
  isValidLinkPreviewSnapshotCanonicalUrl,
} from "@/shared/lib/linkPreviewSnapshot";
import {
  loadLinkPreviewMetadata,
  resolveLinkPreview,
} from "@/shared/lib/useResolvedLinkPreviews";

const POST_SUBMIT_PREVIEW_BUDGET_MS = 10_000;
const SETTLED_PREVIEW_JOB_TTL_MS = 5 * 60_000;

type PreviewJob = {
  controller: AbortController;
  promise: Promise<string[] | null>;
  fingerprint: string | null;
  fallbackTag: string[] | null;
  resolvedTag: string[] | null;
  settled: boolean;
  settledAt: number | null;
};

type BackgroundPreviewTask = {
  cancel: () => void;
  id: number;
  skip: () => void;
};

type BackgroundPreviewSnapshot = {
  canSkip: boolean;
  isPreparing: boolean;
};

export type BackgroundLinkPreviewResult =
  | { status: "cancelled" }
  | { status: "ready"; tags: string[][] };

export type PreparedBackgroundLinkPreviews = {
  cancel: () => void;
  promise: Promise<BackgroundLinkPreviewResult>;
  signal: AbortSignal;
  release: () => void;
  skip: () => void;
};

const jobs = new Map<string, PreviewJob>();
const tasks = new Map<number, BackgroundPreviewTask>();
const promotedSends = new Map<number, AbortController>();
const listeners = new Set<() => void>();
let nextTaskId = 0;
let nextPromotedSendId = 0;
let nextUploadId = 0;
let snapshot: BackgroundPreviewSnapshot = {
  canSkip: false,
  isPreparing: false,
};

function publishSnapshot(): void {
  snapshot = {
    canSkip: tasks.size > 0,
    isPreparing: tasks.size > 0,
  };
  for (const listener of listeners) listener();
}

function dataUrlBytes(dataUrl: string | null | undefined): Uint8Array | null {
  if (!dataUrl) return null;
  const comma = dataUrl.indexOf(",");
  if (comma < 0) return null;
  try {
    const binary = atob(dataUrl.slice(comma + 1));
    return Uint8Array.from(binary, (character) => character.charCodeAt(0));
  } catch {
    return null;
  }
}

async function uploadDataUrl(
  dataUrl: string | null | undefined,
  filename: string,
  signal: AbortSignal,
): Promise<{ failed: boolean; sha256: string; url: string }> {
  const bytes = dataUrlBytes(dataUrl);
  if (!bytes) return { failed: false, sha256: "", url: "" };
  if (signal.aborted) return { failed: true, sha256: "", url: "" };

  const progressId = `link-preview-${nextUploadId++}`;
  let cancellation: Promise<void> | null = null;
  const cancel = () => {
    cancellation ??= cancelMediaUpload(progressId).catch(() => undefined);
  };
  signal.addEventListener("abort", cancel, { once: true });
  try {
    const uploaded = await uploadMediaBytes([...bytes], filename, progressId);
    if (signal.aborted) return { failed: true, sha256: "", url: "" };
    return { failed: false, sha256: uploaded.sha256, url: uploaded.url };
  } catch {
    return { failed: true, sha256: "", url: "" };
  } finally {
    signal.removeEventListener("abort", cancel);
    if (cancellation) await cancellation;
    await releaseMediaUpload(progressId).catch(() => undefined);
  }
}

async function buildSnapshot(
  candidate: SupportedLinkPreview,
  signal: AbortSignal,
  onMetadataReady: (tag: string[]) => void,
): Promise<string[] | null> {
  const metadata = await loadLinkPreviewMetadata(candidate.href);
  if (signal.aborted || !metadata) return null;
  const preview = resolveLinkPreview(candidate, metadata);
  if (!preview.snapshotReady) return null;
  const fallbackTag = buildLinkPreviewSnapshotTag({
    canonicalUrl: preview.href,
    title: preview.title,
    siteName: preview.provider,
    description: preview.description ?? "",
    imageUrl: "",
    imageSha256: "",
    faviconUrl: "",
    faviconSha256: "",
  });
  if (!fallbackTag || signal.aborted) return null;
  onMetadataReady(fallbackTag);
  const [image, favicon] = await Promise.all([
    uploadDataUrl(preview.imageDataUrl, "link-preview-image.png", signal),
    uploadDataUrl(preview.faviconDataUrl, "link-preview-favicon.png", signal),
  ]);
  if (signal.aborted) return null;
  if (image.failed || favicon.failed) return fallbackTag;
  return (
    buildLinkPreviewSnapshotTag({
      canonicalUrl: preview.href,
      title: preview.title,
      siteName: preview.provider,
      description: preview.description ?? "",
      imageUrl: image.url,
      imageSha256: image.sha256,
      faviconUrl: favicon.url,
      faviconSha256: favicon.sha256,
    }) ?? fallbackTag
  );
}

function isReusableJob(job: PreviewJob, now = Date.now()): boolean {
  return (
    !job.settled ||
    (job.settledAt !== null && now - job.settledAt < SETTLED_PREVIEW_JOB_TTL_MS)
  );
}

/**
 * Supersede preparation derived from an older composer incarnation. Deleting
 * before aborting lets a fresh job for the same URL start immediately while
 * the old upload unwinds; the old job's identity check prevents it from
 * deleting or publishing over its replacement.
 */
export function invalidateLinkPreviewPreparation(href: string): void {
  const job = jobs.get(href);
  if (!job) return;
  jobs.delete(href);
  job.controller.abort();
}

function previewFingerprint(candidate: SupportedLinkPreview): string | null {
  if (!("snapshotReady" in candidate) || !candidate.snapshotReady) return null;
  return JSON.stringify([
    candidate.title,
    candidate.provider,
    "description" in candidate ? candidate.description : null,
    "imageDataUrl" in candidate ? candidate.imageDataUrl : null,
    "faviconDataUrl" in candidate ? candidate.faviconDataUrl : null,
  ]);
}

/** Start or adopt the one preparation job for this exact canonical URL. */
export function prepareLinkPreview(
  candidate: SupportedLinkPreview,
): Promise<string[] | null> {
  if (
    candidate.href.startsWith("buzz://") ||
    !isValidLinkPreviewSnapshotCanonicalUrl(candidate.href)
  ) {
    return Promise.resolve(null);
  }
  const fingerprint = previewFingerprint(candidate);
  const existing = jobs.get(candidate.href);
  if (
    existing &&
    isReusableJob(existing) &&
    (fingerprint === null || existing.fingerprint === fingerprint)
  ) {
    return existing.promise;
  }
  if (existing) invalidateLinkPreviewPreparation(candidate.href);

  const controller = new AbortController();
  const job: PreviewJob = {
    controller,
    promise: Promise.resolve(null),
    fingerprint,
    fallbackTag: null,
    resolvedTag: null,
    settled: false,
    settledAt: null,
  };
  job.promise = buildSnapshot(candidate, controller.signal, (fallbackTag) => {
    job.fallbackTag = fallbackTag;
  })
    .catch(() => null)
    .then((tag) => {
      job.resolvedTag = tag;
      if (tag === null && jobs.get(candidate.href) === job) {
        jobs.delete(candidate.href);
      }
      return tag;
    })
    .finally(() => {
      job.settled = true;
      job.settledAt = Date.now();
    });
  jobs.set(candidate.href, job);
  return job.promise;
}

/**
 * Promote the frozen composer generation into a navigation-safe send task.
 * Preparation is best effort: Skip, timeout, or failure all authorize the
 * already-requested send without previews.
 */
export function prepareBackgroundLinkPreviews(
  candidates: readonly SupportedLinkPreview[],
  timeoutMs = POST_SUBMIT_PREVIEW_BUDGET_MS,
): PreparedBackgroundLinkPreviews | null {
  const external = candidates.filter(
    (candidate) =>
      !candidate.href.startsWith("buzz://") &&
      isValidLinkPreviewSnapshotCanonicalUrl(candidate.href),
  );
  if (external.length === 0) return null;

  const sendId = nextPromotedSendId++;
  const controller = new AbortController();
  promotedSends.set(sendId, controller);
  const release = () => {
    if (promotedSends.get(sendId) === controller) {
      promotedSends.delete(sendId);
    }
  };
  const preparedSend = (
    promise: Promise<BackgroundLinkPreviewResult>,
    skip: () => void,
  ): PreparedBackgroundLinkPreviews => ({
    cancel: () => controller.abort(),
    promise,
    signal: controller.signal,
    release,
    skip,
  });

  const pending = external.some(
    (candidate) => !jobs.get(candidate.href)?.settled,
  );
  if (!pending) {
    return preparedSend(
      Promise.all(external.map(prepareLinkPreview)).then((tags) => ({
        status: "ready" as const,
        tags: tags.filter((tag): tag is string[] => tag !== null),
      })),
      () => undefined,
    );
  }

  const availableTags = () =>
    external.flatMap((candidate) => {
      const job = jobs.get(candidate.href);
      const tag = job?.resolvedTag ?? job?.fallbackTag;
      return tag ? [tag] : [];
    });
  const taskId = nextTaskId++;
  let finish: ((result: BackgroundLinkPreviewResult) => void) | null = null;
  let terminal = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const complete = (result: BackgroundLinkPreviewResult) => {
    if (terminal) return;
    terminal = true;
    if (timer !== null) clearTimeout(timer);
    tasks.delete(taskId);
    publishSnapshot();
    finish?.(result);
  };
  const promise = new Promise<BackgroundLinkPreviewResult>((resolve) => {
    finish = resolve;
  });
  const cancel = () => complete({ status: "cancelled" });
  const skip = () => complete({ status: "ready", tags: [] });
  tasks.set(taskId, { cancel, id: taskId, skip });
  publishSnapshot();

  timer = setTimeout(
    () => complete({ status: "ready", tags: availableTags() }),
    timeoutMs,
  );
  void Promise.all(external.map(prepareLinkPreview)).then((tags) => {
    complete({
      status: "ready",
      tags: tags.filter((tag): tag is string[] => tag !== null),
    });
  });

  return preparedSend(promise, skip);
}

export function skipBackgroundLinkPreviews(): void {
  const latestTask = [...tasks.values()].reduce<
    BackgroundPreviewTask | undefined
  >(
    (latest, task) => (!latest || task.id > latest.id ? task : latest),
    undefined,
  );
  latestTask?.skip();
}

export function resetLinkPreviewPreparations(): void {
  for (const controller of promotedSends.values()) controller.abort();
  promotedSends.clear();
  for (const task of [...tasks.values()]) task.cancel();
  for (const job of jobs.values()) job.controller.abort();
  jobs.clear();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): BackgroundPreviewSnapshot {
  return snapshot;
}

export function useBackgroundLinkPreviewPreparation(): BackgroundPreviewSnapshot {
  return React.useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export const __linkPreviewPreparationTest = {
  isReusableJob,
  jobs,
  reset: resetLinkPreviewPreparations,
};
