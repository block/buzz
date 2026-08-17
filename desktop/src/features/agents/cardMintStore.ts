import * as React from "react";
import { toast } from "sonner";

import {
  mintAgentCard,
  NO_OPENAI_KEY_PREFIX,
  type MintedAgentCard,
  type SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";

/**
 * Module store for agent-card mints (`useSyncExternalStore` pattern, same as
 * `avatarPresentationStore`).
 *
 * A mint is one stateless ~2–3 minute Rust call. Owning the in-flight promise
 * here — instead of inside the mint dialog — is what makes the dialog
 * non-blocking: it dispatches and closes, the composer activity rail shows a
 * live "Minting card…" chip, and completion lands as a clickable toast plus a
 * persistent "card ready" chip, none of which need the dialog mounted.
 */

/** Everything needed to run (or re-run) one mint. */
export type CardMintInput = {
  agentId: string;
  agentName: string;
  styleNotes?: string;
  lock?: boolean;
  /** Memory to embed in the card's snapshot. Omitted = "none". */
  memoryLevel?: SnapshotMemoryLevel;
};

export type CardMintJob = {
  jobId: string;
  input: CardMintInput;
  phase: "minting" | "done" | "error";
  /** Populated when phase is "done". */
  card: MintedAgentCard | null;
  /** Populated when phase is "error". */
  error: string | null;
  startedAt: number;
};

/** Card content shown by the global viewer dialog. */
export type CardViewerState = {
  card: MintedAgentCard;
  agentName: string;
  /**
   * Present when the card can be rerolled (fresh mints carry their input;
   * archive views do not — the original style notes are gone).
   */
  remint: CardMintInput | null;
  /**
   * Monotonic per-open sequence, assigned by the store. The viewer keys its
   * content on this so switching cards remounts (resetting recipients/menu
   * state) — card bytes can't serve as the key because every card PNG shares
   * the same header prefix and dimensions.
   */
  viewerSeq: number;
};

let jobs: CardMintJob[] = [];
let viewer: CardViewerState | null = null;
let galleryOpen = false;
const listeners = new Set<() => void>();
let nextJobId = 1;
let nextViewerSeq = 1;

function emitChange(): void {
  for (const listener of listeners) listener();
}

function updateJob(jobId: string, patch: Partial<CardMintJob>): void {
  jobs = jobs.map((job) => (job.jobId === jobId ? { ...job, ...patch } : job));
  emitChange();
}

/**
 * Run one mint as a background job. `mintFn` is injectable for tests; the
 * public `startCardMint` binds the real Tauri command.
 */
export async function runCardMintJob(
  input: CardMintInput,
  mintFn: (
    id: string,
    styleNotes?: string,
    lock?: boolean,
    memoryLevel?: SnapshotMemoryLevel,
  ) => Promise<MintedAgentCard>,
): Promise<void> {
  const jobId = `card-mint-${nextJobId++}`;
  jobs = [
    ...jobs,
    {
      jobId,
      input,
      phase: "minting",
      card: null,
      error: null,
      startedAt: Date.now(),
    },
  ];
  emitChange();

  try {
    const card = await mintFn(
      input.agentId,
      input.styleNotes,
      input.lock,
      input.memoryLevel,
    );
    updateJob(jobId, { phase: "done", card });
    toast.success(`${input.agentName}'s card is ready`, {
      action: {
        label: "View card",
        onClick: () => viewMintedCardJob(jobId),
      },
      duration: 10_000,
    });
  } catch (error) {
    let message = error instanceof Error ? error.message : "Card mint failed.";
    if (message.startsWith(NO_OPENAI_KEY_PREFIX)) {
      // The dialog pre-checks the key, so this only happens when the key was
      // removed between dialog-open and mint. The dialog's key-setup panel is
      // long gone — surface a plain instruction instead of the wire prefix.
      message = message.slice(NO_OPENAI_KEY_PREFIX.length).trim();
    } else if (
      message.startsWith("Card mint failed (HTTP 401 ") ||
      message.includes("Incorrect API key")
    ) {
      // The saved OpenAI key is invalid or expired. Only match the OpenAI-call
      // envelope prefix and the specific Incorrect-API-key message to avoid
      // rewriting unrelated 401s (e.g. "Avatar fetch failed: HTTP 401 …").
      message =
        'The OpenAI API key is invalid or expired. Open the mint dialog and use "Update API key" to replace it.';
    }
    updateJob(jobId, { phase: "error", error: message });
    toast.error(`Minting ${input.agentName}'s card failed`, {
      description: message,
    });
  }
}

/** Start a mint in the background. Fire-and-forget; state flows via the store. */
export function startCardMint(input: CardMintInput): void {
  void runCardMintJob(input, mintAgentCard);
}

/** Open the finished card of a job in the viewer and clear its rail chip. */
export function viewMintedCardJob(jobId: string): void {
  const job = jobs.find((candidate) => candidate.jobId === jobId);
  if (job?.phase !== "done" || !job.card) return;
  viewer = {
    card: job.card,
    agentName: job.input.agentName,
    remint: job.input,
    viewerSeq: nextViewerSeq++,
  };
  jobs = jobs.filter((candidate) => candidate.jobId !== jobId);
  emitChange();
}

/** Remove a job chip (used for error dismissal). */
export function dismissCardMintJob(jobId: string): void {
  const next = jobs.filter((candidate) => candidate.jobId !== jobId);
  if (next.length === jobs.length) return;
  jobs = next;
  emitChange();
}

/** Open the viewer on an arbitrary card (e.g. one loaded from the archive). */
export function openCardViewer(
  state: Omit<CardViewerState, "viewerSeq">,
): void {
  viewer = { ...state, viewerSeq: nextViewerSeq++ };
  emitChange();
}

export function closeCardViewer(): void {
  if (!viewer) return;
  viewer = null;
  emitChange();
}

export function setCardGalleryOpen(open: boolean): void {
  if (galleryOpen === open) return;
  galleryOpen = open;
  emitChange();
}

export function resetCardMintStore(): void {
  jobs = [];
  viewer = null;
  galleryOpen = false;
  emitChange();
}

export function getCardMintJobs(): CardMintJob[] {
  return jobs;
}

export function getCardViewerState(): CardViewerState | null {
  return viewer;
}

export function getCardGalleryOpen(): boolean {
  return galleryOpen;
}

export function subscribeCardMintStore(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useCardMintJobs(): CardMintJob[] {
  return React.useSyncExternalStore(
    subscribeCardMintStore,
    getCardMintJobs,
    getCardMintJobs,
  );
}

export function useCardViewerState(): CardViewerState | null {
  return React.useSyncExternalStore(
    subscribeCardMintStore,
    getCardViewerState,
    getCardViewerState,
  );
}

export function useCardGalleryOpen(): boolean {
  return React.useSyncExternalStore(
    subscribeCardMintStore,
    getCardGalleryOpen,
    getCardGalleryOpen,
  );
}
