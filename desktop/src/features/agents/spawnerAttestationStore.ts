import React from "react";
import { toast } from "sonner";

import { getIdentity } from "@/shared/api/tauriIdentity";
import {
  setManagedAgentRelocated,
  stopManagedAgent,
} from "@/shared/api/tauriManagedAgents";
import {
  respondToSpawnerAttestation,
  subscribeToSpawnerAttestations,
} from "@/shared/api/spawnerRelay";
import {
  buildSpawnerAttestationResponse,
  decodeSpawnerAttestation,
  type SpawnerAttestationRequest,
  type SpawnerAttestationResponse,
  type SpawnerPromptMaterial,
} from "@/shared/api/tauriSpawner";
import type { RelayEvent } from "@/shared/api/types";
import { isSpawnerTrusted, trustSpawner } from "./trustedSpawners";

/**
 * Inbound kind:24201 attestation requests awaiting the owner's decision.
 *
 * A module-level singleton rather than React state, matching
 * `observerRelayStore`: the relay subscription must outlive any one component,
 * and the queue has to survive navigation so a request that arrives while the
 * user is elsewhere is not lost.
 *
 * # Auto-approval
 *
 * A request from an already-trusted spawner is signed without prompting. That
 * is the whole point of `trustedSpawners` — an owner running five agents on
 * their own VPS approves the spawner once, not five times. Requests from an
 * unknown spawner always queue for a human.
 */

/** A pending request, carrying the ciphertext needed to answer it. */
export type PendingAttestation = SpawnerAttestationRequest & {
  /** Original NIP-44 ciphertext, re-decrypted in Rust when responding. */
  encryptedContent: string;
  /** Relay event id, used to deduplicate replays on reconnect. */
  eventId: string;
};

const listeners = new Set<() => void>();

let pending: readonly PendingAttestation[] = [];
let unsubscribeRelay: (() => Promise<void>) | null = null;
let startPromise: Promise<void> | null = null;

/**
 * Event ids already handled.
 *
 * The subscription uses a `since` lookback, so a reconnect can redeliver a
 * frame that was already answered. Without this the owner would be prompted
 * twice for one agent, and the second answer would be rejected by the spawner
 * anyway (its nonce is no longer in flight).
 */
const handledEventIds = new Set<string>();

/**
 * Resolves prompt material for a spec slug at approval time.
 *
 * Injected rather than imported so this store stays free of the persona layer.
 * `useSpawnerIngestion` wires the real resolver; until it does, approvals send
 * no prompt and a spawner falls back to reading a shared persona.
 */
let promptResolver:
  | ((specSlug: string) => SpawnerPromptMaterial | null)
  | null = null;

/** Register the prompt resolver used when answering an attestation. */
export function setSpawnerPromptResolver(
  resolver: ((specSlug: string) => SpawnerPromptMaterial | null) | null,
): void {
  promptResolver = resolver;
}

function promptFor(specSlug: string): SpawnerPromptMaterial | undefined {
  return promptResolver?.(specSlug) ?? undefined;
}

/**
 * Publish an attestation answer, then retire the local copy of a relocated
 * agent.
 *
 * The order is deliberate and load-bearing:
 *
 * 1. **Publish first.** If the response never reaches the spawner the agent is
 *    still only running here, and stopping it first would take the user's agent
 *    offline everywhere for nothing.
 * 2. **Mark relocated second.** Relocation must be *state*, not just a stop:
 *    the auto-restart policy, app-launch restore, runtime reconcile, and the
 *    manual Start button all resurrect a merely-stopped agent (the split-brain
 *    found in testing came from exactly that). The persisted flag is what every
 *    start path checks, so it lands before the stop it protects.
 * 3. **Stop third, and only on `relocatedAgentPubkey`.** Rust sets that field
 *    exactly when it handed the agent's secret key to the spawner, so from that
 *    moment two processes hold one key. Both would see every mention and both
 *    would reply: duplicate answers, and the owner billed twice per turn.
 *
 * A failed mark or stop is surfaced as a toast rather than swallowed. It leaves
 * a real split brain that only the user can resolve, so silence is the one
 * unacceptable outcome — the publish itself already succeeded and must not be
 * retried.
 */
async function publishAndRetireRelocated(
  response: SpawnerAttestationResponse,
  spawnerPubkey: string,
): Promise<void> {
  await respondToSpawnerAttestation(response.event);

  const relocated = response.relocatedAgentPubkey;
  if (!relocated) return;
  try {
    await setManagedAgentRelocated(relocated, spawnerPubkey);
    await stopManagedAgent(relocated);
  } catch (error) {
    toast.error(
      `This agent now runs on the server, but the copy on this Mac could not be retired: ${
        error instanceof Error ? error.message : "unknown error"
      }. Stop it from the Agents screen — until you do, it will answer twice.`,
      { duration: Number.POSITIVE_INFINITY },
    );
  }
}

/** Bound on `handledEventIds` so a long session cannot grow it without limit. */
const MAX_HANDLED_IDS = 500;

const EMPTY: readonly PendingAttestation[] = [];

function notify(): void {
  for (const listener of listeners) listener();
}

function markHandled(eventId: string): void {
  handledEventIds.add(eventId);
  if (handledEventIds.size > MAX_HANDLED_IDS) {
    // Insertion-ordered: dropping the oldest keeps the window covering recent
    // frames, which is all the lookback can redeliver.
    const oldest = handledEventIds.values().next().value;
    if (oldest !== undefined) handledEventIds.delete(oldest);
  }
}

function removePending(nonce: string): void {
  const next = pending.filter((item) => item.nonce !== nonce);
  if (next.length !== pending.length) {
    pending = next;
    notify();
  }
}

async function handleAttestationEvent(event: RelayEvent): Promise<void> {
  if (handledEventIds.has(event.id)) return;
  markHandled(event.id);

  let request: SpawnerAttestationRequest | null;
  try {
    request = await decodeSpawnerAttestation(event.pubkey, event.content);
  } catch {
    // Not addressed to us, malformed, or not decryptable with our key. Anyone
    // can publish a p-tagged frame, so this is expected traffic, not an error
    // worth surfacing.
    return;
  }
  if (!request) return;

  // The verified event author is the spawner. The frame body does not name its
  // own sender, so there is nothing to cross-check here — Rust decrypts with
  // this pubkey, meaning a frame that decodes at all came from it.
  if (request.spawnerPubkey !== event.pubkey) return;

  if (isSpawnerTrusted(request.spawnerPubkey)) {
    try {
      await publishAndRetireRelocated(
        await buildSpawnerAttestationResponse({
          spawnerPubkey: request.spawnerPubkey,
          encryptedContent: event.content,
          trust: "trusted",
          prompt: promptFor(request.specSlug),
        }),
        request.spawnerPubkey,
      );
    } catch {
      // Fall through to prompting. A failed auto-approval (relay down,
      // signing unavailable) should surface to the user rather than leaving
      // the agent stuck at pending_attestation with no explanation.
      enqueue(request, event);
    }
    return;
  }

  enqueue(request, event);
}

function enqueue(request: SpawnerAttestationRequest, event: RelayEvent): void {
  // A spawner re-sends with a fresh nonce after a timeout. Replacing the entry
  // for the same agent keeps one prompt per agent rather than stacking stale
  // ones the spawner would reject.
  const withoutSameAgent = pending.filter(
    (item) => item.agentPubkey !== request.agentPubkey,
  );
  pending = [
    ...withoutSameAgent,
    { ...request, encryptedContent: event.content, eventId: event.id },
  ];
  notify();
}

/** Approve a pending request, optionally remembering the spawner. */
export async function approveAttestation(
  item: PendingAttestation,
  options: { remember: boolean },
): Promise<void> {
  await publishAndRetireRelocated(
    await buildSpawnerAttestationResponse({
      spawnerPubkey: item.spawnerPubkey,
      encryptedContent: item.encryptedContent,
      trust: "trusted",
      prompt: promptFor(item.specSlug),
    }),
    item.spawnerPubkey,
  );
  // Remember only after the signature actually went out, so a failed publish
  // does not leave a spawner silently trusted for every future request.
  if (options.remember) trustSpawner(item.spawnerPubkey);
  removePending(item.nonce);
}

/**
 * Decline a pending request.
 *
 * Sends an explicit rejection rather than dropping it silently, so the spawner
 * reports `failed` immediately instead of leaving the agent at
 * `pending_attestation` until its timeout expires.
 */
export async function declineAttestation(
  item: PendingAttestation,
  reason?: string,
): Promise<void> {
  // Deliberately not `publishAndRetireRelocated`: a rejection hands over no
  // key, so nothing may stop a local agent on this path.
  const { event } = await buildSpawnerAttestationResponse({
    spawnerPubkey: item.spawnerPubkey,
    encryptedContent: item.encryptedContent,
    trust: "untrusted",
    rejectReason: reason,
  });
  await respondToSpawnerAttestation(event);
  removePending(item.nonce);
}

/** Open the attestation subscription. Idempotent. */
export async function ensureSpawnerAttestationSubscription(): Promise<void> {
  if (unsubscribeRelay) return;
  if (startPromise) return startPromise;

  startPromise = (async () => {
    const identity = await getIdentity();
    if (!identity?.pubkey) return;
    const dispose = await subscribeToSpawnerAttestations(
      identity.pubkey,
      (event) => {
        void handleAttestationEvent(event);
      },
    );
    // A reset that landed while we were connecting must not leave a live
    // subscription behind for the previous identity.
    if (startPromise === null) {
      void dispose();
      return;
    }
    unsubscribeRelay = dispose;
  })();

  try {
    await startPromise;
  } finally {
    if (unsubscribeRelay) startPromise = null;
  }
}

/**
 * Tear down the subscription and drop queued state.
 *
 * Wired into `resetCommunityState()` — pending prompts belong to the community
 * and identity they arrived under, and showing one after a community switch
 * would ask the user to sign with the wrong key.
 */
export function resetSpawnerAttestationStore(): void {
  const dispose = unsubscribeRelay;
  unsubscribeRelay = null;
  startPromise = null;
  if (dispose) void dispose();

  handledEventIds.clear();
  if (pending.length > 0) {
    pending = [];
    notify();
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): readonly PendingAttestation[] {
  return pending;
}

function getServerSnapshot(): readonly PendingAttestation[] {
  return EMPTY;
}

/** Reactive view of the pending attestation queue. */
export function usePendingAttestations(): readonly PendingAttestation[] {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
