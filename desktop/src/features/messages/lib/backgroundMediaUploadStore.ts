import * as React from "react";

import type { BlobDescriptor } from "@/shared/api/tauri";
import { cancelMediaUpload, uploadMediaFile } from "@/shared/api/tauriMedia";
import {
  type BackgroundMediaUploadPhase,
  isNativeMediaUploadPhase,
  resolveBackgroundMediaUploadPhase,
} from "./backgroundMediaUploadPhase";

export type QueuedMediaAttachment = {
  file: File;
  id: number;
  previewUrl?: string;
  spoilered: boolean;
};

type BackgroundUploadTask = {
  abortController: AbortController;
  canceled: boolean;
  filePhases: BackgroundMediaUploadPhase[];
  fileProgress: Array<{ sent: number; total: number }>;
  id: number;
  isCompleting: boolean;
  onCancel?: () => void;
};

type BackgroundUploadSnapshot = {
  canCancel: boolean;
  isUploading: boolean;
  phase: BackgroundMediaUploadPhase;
  percentage: number;
};

type EnqueueBackgroundUploadOptions = {
  attachments: QueuedMediaAttachment[];
  onCancel?: () => void;
  onComplete: (
    descriptors: BlobDescriptor[],
    signal: AbortSignal,
  ) => Promise<void>;
  onError: (error: unknown) => void;
};

type StartBackgroundUploadOptions = Omit<
  EnqueueBackgroundUploadOptions,
  "attachments"
>;

export type PreparedBackgroundMediaUpload = {
  cancel: () => void;
  start: (options: StartBackgroundUploadOptions) => boolean;
};

const tasks = new Map<number, BackgroundUploadTask>();
const queuedAttachmentsByDraftKey = new Map<string, QueuedMediaAttachment[]>();
const listeners = new Set<() => void>();
let nextTaskId = 0;
let snapshot: BackgroundUploadSnapshot = {
  canCancel: false,
  isUploading: false,
  phase: "preparing",
  percentage: 0,
};
let stopUploadListeners: (() => void)[] = [];
let uploadListenersPromise: Promise<void> | null = null;

function progressId(taskId: number, fileIndex: number): string {
  return `background-media-upload-${taskId}-${fileIndex}`;
}

function rebuildSnapshot(): void {
  const allTasks = [...tasks.values()];
  const allProgress = allTasks.flatMap((task) => task.fileProgress);
  const totalBytes = allProgress.reduce(
    (total, progress) => total + progress.total,
    0,
  );
  const sentBytes = allProgress.reduce(
    (total, progress) => total + progress.sent,
    0,
  );
  snapshot = {
    canCancel: allTasks.some((task) => !task.isCompleting),
    isUploading: allTasks.length > 0,
    phase: resolveBackgroundMediaUploadPhase(
      allTasks.flatMap((task) => task.filePhases),
    ),
    percentage:
      totalBytes === 0 ? 0 : Math.round((sentBytes / totalBytes) * 100),
  };
  for (const listener of listeners) listener();
}

async function ensureUploadListeners(): Promise<void> {
  if (stopUploadListeners.length > 0) return;
  if (uploadListenersPromise) {
    await uploadListenersPromise;
    return;
  }
  uploadListenersPromise = (async () => {
    const disposers: (() => void)[] = [];
    try {
      const { listen } = await import("@tauri-apps/api/event");
      disposers.push(
        await listen<{
          id: string;
          sent: number;
          total: number;
        }>("media-upload-progress", (event) => {
          const match = /^background-media-upload-(\d+)-(\d+)$/.exec(
            event.payload.id,
          );
          if (!match || event.payload.total <= 0) return;

          const task = tasks.get(Number(match[1]));
          const fileIndex = Number(match[2]);
          if (!task || fileIndex >= task.fileProgress.length) return;

          task.filePhases[fileIndex] = "uploading";
          task.fileProgress[fileIndex] = {
            sent: Math.min(
              event.payload.total,
              Math.max(0, event.payload.sent),
            ),
            total: event.payload.total,
          };
          rebuildSnapshot();
        }),
      );
      disposers.push(
        await listen<{ id: string; phase: unknown }>(
          "media-upload-phase",
          (event) => {
            const match = /^background-media-upload-(\d+)-(\d+)$/.exec(
              event.payload.id,
            );
            if (!match || !isNativeMediaUploadPhase(event.payload.phase)) {
              return;
            }

            const task = tasks.get(Number(match[1]));
            const fileIndex = Number(match[2]);
            if (!task || fileIndex >= task.filePhases.length) return;

            task.filePhases[fileIndex] = event.payload.phase;
            rebuildSnapshot();
          },
        ),
      );
      if (tasks.size === 0) {
        for (const dispose of disposers) dispose();
      } else {
        stopUploadListeners = disposers;
      }
    } catch {
      for (const dispose of disposers) dispose();
      // Browser and E2E runtimes do not emit native upload state.
    } finally {
      uploadListenersPromise = null;
    }
  })();
  await uploadListenersPromise;
}

function finishTask(taskId: number): void {
  tasks.delete(taskId);
  rebuildSnapshot();
  if (tasks.size === 0) {
    for (const dispose of stopUploadListeners) dispose();
    stopUploadListeners = [];
  }
}

function cancelTask(
  task: BackgroundUploadTask,
  { force = false, notify = true }: { force?: boolean; notify?: boolean } = {},
): void {
  if (task.canceled || (task.isCompleting && !force)) return;
  task.canceled = true;
  task.abortController.abort();
  if (notify) task.onCancel?.();
  for (let index = 0; index < task.fileProgress.length; index += 1) {
    void cancelMediaUpload(progressId(task.id, index)).catch(() => undefined);
  }
  finishTask(task.id);
}

function yieldForUploadFeedback(): Promise<void> {
  if (
    typeof window === "undefined" ||
    typeof window.requestAnimationFrame !== "function" ||
    document.visibilityState === "hidden"
  ) {
    return new Promise((resolve) => setTimeout(resolve, 0));
  }

  return new Promise((resolve) => {
    window.requestAnimationFrame(() => window.setTimeout(resolve, 0));
  });
}

export function prepareBackgroundMediaUpload(
  attachments: QueuedMediaAttachment[],
): PreparedBackgroundMediaUpload {
  if (attachments.length === 0) {
    let started = false;
    return {
      cancel: () => undefined,
      start: ({ onComplete, onError }) => {
        if (started) return false;
        started = true;
        void onComplete([], new AbortController().signal).catch(onError);
        return true;
      },
    };
  }

  const taskId = nextTaskId;
  nextTaskId += 1;
  const task: BackgroundUploadTask = {
    abortController: new AbortController(),
    canceled: false,
    filePhases: attachments.map(() => "preparing"),
    fileProgress: attachments.map((attachment) => ({
      sent: 0,
      total: attachment.file.size,
    })),
    id: taskId,
    isCompleting: false,
  };
  let started = false;
  tasks.set(taskId, task);
  rebuildSnapshot();

  return {
    cancel: () => {
      cancelTask(task);
    },
    start: ({ onCancel, onComplete, onError }) => {
      if (started || task.canceled) return false;
      started = true;
      task.onCancel = onCancel;

      void (async () => {
        try {
          await ensureUploadListeners();
          // Let React commit and paint the 0% task before file reads or native
          // IPC begin, so large attachments never hide the initial feedback.
          await yieldForUploadFeedback();
          const descriptors: BlobDescriptor[] = [];
          for (let index = 0; index < attachments.length; index += 1) {
            if (task.canceled) return;
            const attachment = attachments[index];
            const descriptor = await uploadMediaFile(
              attachment.file,
              progressId(taskId, index),
              task.abortController.signal,
            );
            if (task.canceled) return;
            task.filePhases[index] = "finishing";
            task.fileProgress[index] = {
              sent: task.fileProgress[index].total,
              total: task.fileProgress[index].total,
            };
            rebuildSnapshot();
            descriptors.push(descriptor);
          }

          if (!task.canceled) {
            task.isCompleting = true;
            task.filePhases.fill("finishing");
            rebuildSnapshot();
            await onComplete(descriptors, task.abortController.signal);
          }
        } catch (error) {
          if (!task.canceled) onError(error);
        } finally {
          finishTask(taskId);
        }
      })();
      return true;
    },
  };
}

export function enqueueBackgroundMediaUpload({
  attachments,
  onCancel,
  onComplete,
  onError,
}: EnqueueBackgroundUploadOptions): PreparedBackgroundMediaUpload {
  const preparedUpload = prepareBackgroundMediaUpload(attachments);
  preparedUpload.start({ onCancel, onComplete, onError });
  return preparedUpload;
}

export function cancelBackgroundMediaUploads(): void {
  for (const task of [...tasks.values()].reverse()) {
    if (!task.isCompleting) {
      cancelTask(task);
      return;
    }
  }
}

export function resetBackgroundMediaUploads(): void {
  for (const task of [...tasks.values()]) {
    cancelTask(task, { force: true, notify: false });
  }
  queuedAttachmentsByDraftKey.clear();
}

/**
 * Retain local files that cannot be serialized with a draft while a deferred
 * upload recovers after the user has left its channel.
 */
export function saveQueuedAttachmentsForDraft(
  draftKey: string,
  attachments: QueuedMediaAttachment[],
): void {
  queuedAttachmentsByDraftKey.set(draftKey, attachments);
}

/** Remove local files retained for a draft without restoring them. */
export function discardQueuedAttachmentsForDraft(draftKey: string): void {
  queuedAttachmentsByDraftKey.delete(draftKey);
}

/** Return and remove the local files retained for a recovered draft. */
export function takeQueuedAttachmentsForDraft(
  draftKey: string,
): QueuedMediaAttachment[] {
  const attachments = queuedAttachmentsByDraftKey.get(draftKey) ?? [];
  queuedAttachmentsByDraftKey.delete(draftKey);
  return attachments;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): BackgroundUploadSnapshot {
  return snapshot;
}

export function useBackgroundMediaUpload(): BackgroundUploadSnapshot {
  return React.useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
