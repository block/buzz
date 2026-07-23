import * as React from "react";
import { toast } from "sonner";

import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

export type AvatarPresentationState = "failed" | "pending" | "ready";

export type AvatarPresentation = {
  displayUrl: string;
  state: AvatarPresentationState;
};

type AvatarPresentationEntry = {
  generation: number;
  localPreviewUrl: string | null;
  remoteUrl: string;
  snapshot: AvatarPresentation;
};

const PROBE_DELAYS_MS = [0, 750, 1_500, 3_000] as const;
const PROBE_TIMEOUT_MS = 3_000;
const READY_PRESENTATION_TTL_MS = 30_000;
const presentations = new Map<string, AvatarPresentationEntry>();
const listeners = new Set<() => void>();
let nextGeneration = 1;

function emitChange(): void {
  for (const listener of listeners) listener();
}

function toastId(remoteUrl: string): string {
  return `avatar-presentation:${remoteUrl}`;
}

function releaseLocalPreview(entry: AvatarPresentationEntry): void {
  if (!entry.localPreviewUrl) return;
  URL.revokeObjectURL(entry.localPreviewUrl);
  entry.localPreviewUrl = null;
}

function isCurrent(entry: AvatarPresentationEntry): boolean {
  return presentations.get(entry.remoteUrl)?.generation === entry.generation;
}

function wait(delayMs: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, delayMs));
}

function buildProbeUrl(remoteUrl: string, attempt: number): string {
  try {
    const url = new URL(remoteUrl);
    url.searchParams.set(
      "buzz_avatar_probe",
      `${Date.now()}-${attempt.toString()}`,
    );
    return url.toString();
  } catch {
    return remoteUrl;
  }
}

function probeImage(
  remoteUrl: string,
  attempt: number,
): Promise<string | null> {
  return new Promise((resolve) => {
    const image = new Image();
    let settled = false;
    const verifiedUrl = buildProbeUrl(remoteUrl, attempt);
    const finish = (result: string | null) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timeoutId);
      image.onload = null;
      image.onerror = null;
      resolve(result);
    };
    const timeoutId = window.setTimeout(() => finish(null), PROBE_TIMEOUT_MS);

    image.onload = () => finish(verifiedUrl);
    image.onerror = () => finish(null);
    image.referrerPolicy = "no-referrer";
    image.src = rewriteRelayUrl(verifiedUrl);
  });
}

async function verifyPresentation(
  entry: AvatarPresentationEntry,
): Promise<void> {
  for (const [attempt, delayMs] of PROBE_DELAYS_MS.entries()) {
    await wait(delayMs);
    if (!isCurrent(entry) || entry.snapshot.state !== "pending") return;

    const verifiedUrl = await probeImage(entry.remoteUrl, attempt);
    if (!isCurrent(entry) || entry.snapshot.state !== "pending") return;
    if (!verifiedUrl) continue;

    releaseLocalPreview(entry);
    entry.snapshot = { displayUrl: verifiedUrl, state: "ready" };
    toast.dismiss(toastId(entry.remoteUrl));
    emitChange();
    window.setTimeout(() => {
      if (!isCurrent(entry) || entry.snapshot.state !== "ready") return;
      presentations.delete(entry.remoteUrl);
      emitChange();
    }, READY_PRESENTATION_TTL_MS);
    return;
  }

  if (!isCurrent(entry) || entry.snapshot.state !== "pending") return;
  entry.snapshot = {
    displayUrl: entry.remoteUrl,
    state: "failed",
  };
  emitChange();
  toast.error("Avatar couldn’t finish uploading", {
    action: {
      label: "Retry",
      onClick: () => retryAvatarPresentation(entry.remoteUrl),
    },
    description: "Your default avatar is showing instead.",
    id: toastId(entry.remoteUrl),
  });
}

export function beginAvatarPresentation(remoteUrl: string, image: Blob): void {
  const existing = presentations.get(remoteUrl);
  if (existing) {
    toast.dismiss(toastId(remoteUrl));
    releaseLocalPreview(existing);
  }

  const localPreviewUrl = URL.createObjectURL(image);
  const entry: AvatarPresentationEntry = {
    generation: nextGeneration++,
    localPreviewUrl,
    remoteUrl,
    snapshot: { displayUrl: localPreviewUrl, state: "pending" },
  };
  presentations.set(remoteUrl, entry);
  emitChange();
  void verifyPresentation(entry);
}

export function disposeAvatarPresentation(remoteUrl: string): void {
  const entry = presentations.get(remoteUrl);
  if (!entry) return;

  entry.generation = nextGeneration++;
  toast.dismiss(toastId(remoteUrl));
  releaseLocalPreview(entry);
  presentations.delete(remoteUrl);
  emitChange();
}

export function resetAvatarPresentations(): void {
  for (const entry of presentations.values()) {
    entry.generation = nextGeneration++;
    toast.dismiss(toastId(entry.remoteUrl));
    releaseLocalPreview(entry);
  }
  presentations.clear();
  emitChange();
}

export function retryAvatarPresentation(remoteUrl: string): void {
  const entry = presentations.get(remoteUrl);
  if (entry?.snapshot.state !== "failed") return;
  entry.generation = nextGeneration++;
  entry.snapshot = {
    displayUrl: entry.localPreviewUrl ?? entry.remoteUrl,
    state: "pending",
  };
  emitChange();
  void verifyPresentation(entry);
}

export function getAvatarPresentation(
  remoteUrl: string | null,
): AvatarPresentation | null {
  if (!remoteUrl) return null;
  return presentations.get(remoteUrl)?.snapshot ?? null;
}

export function subscribeAvatarPresentations(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useAvatarPresentation(
  remoteUrl: string | null,
): AvatarPresentation | null {
  return React.useSyncExternalStore(
    subscribeAvatarPresentations,
    () => getAvatarPresentation(remoteUrl),
    () => null,
  );
}

export function useAvatarSelection(
  avatarUrl: string,
  onUrlChange: (avatarUrl: string) => void,
): (avatarUrl: string) => void {
  return React.useCallback(
    (nextAvatarUrl: string) => {
      if (avatarUrl !== nextAvatarUrl) disposeAvatarPresentation(avatarUrl);
      onUrlChange(nextAvatarUrl);
    },
    [avatarUrl, onUrlChange],
  );
}
