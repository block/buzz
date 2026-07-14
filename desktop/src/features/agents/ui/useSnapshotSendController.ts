/**
 * useSnapshotSendController
 *
 * Payload-agnostic upload → send controller for sharing a snapshot to a Buzz
 * channel or DM.  The caller supplies an encode function and a destination;
 * the controller drives prepare → encode → upload → send with honest progress,
 * idempotent double-send protection, and fail-closed eligibility checks at two
 * action-boundary checkpoints that read from live query-cache sources, not from
 * render-captured snapshots.
 *
 * This hook does not know what kind of snapshot the bytes contain.  A future
 * team-snapshot or other payload can reuse it unchanged by passing different
 * bytes and a filename.  Hard-coded semantics for `.agent.*` live only in
 * the export-dialog layer above this hook.
 */

import * as React from "react";
import type { QueryClient } from "@tanstack/react-query";
import { useQueryClient } from "@tanstack/react-query";

import { uploadMediaBytes, type BlobDescriptor } from "@/shared/api/tauri";
import { buildOutgoingMessage } from "@/features/messages/lib/imetaMediaMarkdown";
import { channelsQueryKey, useChannelsQuery } from "@/features/channels/hooks";
import { isModerationDm } from "@/features/moderation/lib/moderationDm";
import {
  relaySelfQueryKey,
  useRelaySelfQuery,
} from "@/features/moderation/hooks";
import { getTimeoutSnapshot } from "@/features/moderation/lib/timeoutStore";
import { isTimeoutActive } from "@/features/moderation/lib/timeout";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import type { Channel, Identity } from "@/shared/api/types";

// ── Public types ──────────────────────────────────────────────────────────────

export type SendPhase =
  | "idle"
  | "preparing"
  | "uploading"
  | "sending"
  | "done"
  | "error";

export type SnapshotSendState = {
  phase: SendPhase;
  error: string | null;
};

/**
 * A channel annotated with a resolved display label.  For non-DM channels the
 * label equals `ch.name`; for DMs it resolves participant display names so the
 * picker, memory-gate warning, and success copy are consistent.
 */
export type ResolvedChannel = Channel & {
  /** Human-readable label for the channel (participant names for DMs). */
  displayLabel: string;
};

/**
 * A joined, non-archived, non-moderation-DM destination: channelType "stream"
 * or "dm", isMember true, archivedAt null.
 *
 * Moderation DM exclusion requires the relay `self` pubkey and the current
 * user pubkey; those are applied in `useSendableChannels` below so callers
 * always receive a fully-filtered list.
 */
export function isSendableDestination(ch: Channel): boolean {
  return ch.isMember && ch.archivedAt === null && ch.channelType !== "forum";
}

/**
 * Pure factory for a single-concurrency action guard.
 *
 * Returns `{ runGuarded }` where `runGuarded(action)` executes `action()`
 * only when no other call is currently in flight; any concurrent call receives
 * `false` immediately.  The guard is the same mechanism used by
 * `beginSend` in `useSnapshotSendController` — exported so unit tests can
 * exercise the production guard logic directly without requiring a React
 * rendering context.
 *
 * @example
 * ```ts
 * const { runGuarded } = createSendGuard();
 * const [r1, r2] = await Promise.all([
 *   runGuarded(async () => { ...encode/upload/send... }),
 *   runGuarded(async () => { ...encode/upload/send... }),
 * ]);
 * // r1 === true (ran), r2 === false (blocked)
 * ```
 */
export function createSendGuard(): {
  runGuarded: (action: () => Promise<boolean>) => Promise<boolean>;
  get inFlight(): boolean;
} {
  let inFlight = false;
  return {
    runGuarded: async (action) => {
      if (inFlight) return false;
      inFlight = true;
      try {
        return await action();
      } finally {
        inFlight = false;
      }
    },
    get inFlight() {
      return inFlight;
    },
  };
}

/**
 * Read current eligibility for `channelId` directly from live query-cache
 * sources and the timeout external store.  Does NOT read rendered React state
 * or component refs; safe to call inside a `runGuarded` action where render
 * state may be stale.
 *
 * Returns `null` when the channel is eligible; returns a human-readable error
 * string when it is not.
 *
 * Fail-closed on DM classification: a DM destination is blocked unless both
 * identity and relay-self have successfully resolved (`status === "success"`).
 * This covers pending, fetching, absent (never fetched), and errored states.
 * The semantic distinction is preserved: a successfully resolved
 * `relaySelf === null` means the relay advertises no self pubkey — that IS a
 * known result and the generic moderation helper's fail-open behavior applies.
 * Any other state (absent, errored, or in-flight) is unknown and must block.
 */
export function checkSendEligibility(
  queryClient: QueryClient,
  channelId: string,
  nowMs: number = Date.now(),
): string | null {
  // ── Timeout check ─────────────────────────────────────────────────────────
  // Read the module-level snapshot from timeoutStore directly — this is the
  // same value `useTimeoutState` serves but without requiring a render cycle.
  const timeoutState = getTimeoutSnapshot();
  if (timeoutState.active && isTimeoutActive(timeoutState.expiresAtMs, nowMs)) {
    return "You are currently timed out and cannot send messages.";
  }

  // ── Channel-cache check ───────────────────────────────────────────────────
  const channels = queryClient.getQueryData<Channel[]>(channelsQueryKey) ?? [];
  const channel = channels.find((ch) => ch.id === channelId);

  if (!channel) {
    return "The selected destination is no longer available. Please pick another.";
  }
  if (!isSendableDestination(channel)) {
    return "The selected destination is no longer available. Please pick another.";
  }

  // ── Moderation-DM check (fail-closed) ────────────────────────────────────
  if (channel.channelType === "dm") {
    const identityState = queryClient.getQueryState<Identity>(["identity"]);
    const relaySelfState = queryClient.getQueryState<string | null>(
      relaySelfQueryKey,
    );

    // Fail-closed: block the DM unless both identity AND relay-self have
    // successfully resolved.  Absent state (undefined — never fetched or
    // cache evicted), pending/fetching, and errored states are all unknown
    // and must block to prevent misclassification.
    //
    // The only exception is a successfully resolved `relaySelf === null`:
    // that means the relay advertises no self pubkey (a valid, known answer),
    // and we pass it to isModerationDm, which fails open by its own contract.
    if (identityState?.status !== "success") {
      return "The selected destination is no longer available. Please pick another.";
    }
    if (relaySelfState?.status !== "success") {
      return "The selected destination is no longer available. Please pick another.";
    }

    const identity = queryClient.getQueryData<Identity>(["identity"]);
    const relaySelf = queryClient.getQueryData<string | null>(
      relaySelfQueryKey,
    );

    if (isModerationDm(channel, identity?.pubkey, relaySelf ?? undefined)) {
      return "The selected destination is no longer available. Please pick another.";
    }
  }

  return null;
}

/**
 * The core send pipeline: prepare → [eligibility] → encode → [eligibility] →
 * upload → send.  Extracted as a standalone async function so unit tests can
 * import and exercise it directly with injected dependencies — the production
 * hook calls it inside `runGuarded`.
 *
 * Dependencies are injected rather than closed-over from React scope so the
 * function is pure-async and fully testable without a rendering context.
 */
export async function runSendPipeline(deps: {
  encodeFn: () => Promise<{ fileBytes: number[]; fileName: string }>;
  uploadFn: (bytes: number[], filename: string) => Promise<BlobDescriptor>;
  sendFn: (args: {
    channelId: string;
    content: string;
    mediaTags: string[][];
  }) => Promise<unknown>;
  setStateFn: (state: SnapshotSendState) => void;
  buildMessageFn: (descriptor: BlobDescriptor) => {
    content: string;
    mediaTags: string[][] | null | undefined;
  };
  checkEligibilityFn: () => string | null;
  channelId: string;
}): Promise<boolean> {
  const {
    encodeFn,
    uploadFn,
    sendFn,
    setStateFn,
    buildMessageFn,
    checkEligibilityFn,
    channelId,
  } = deps;

  // ── Eligibility checkpoint 1: before encode ───────────────────────────────
  // Reads live sources directly — timeout store, channel cache, identity cache,
  // relay-self cache.  Not a render snapshot.
  const reason1 = checkEligibilityFn();
  if (reason1 !== null) {
    setStateFn({ phase: "error", error: reason1 });
    return false;
  }

  // ── Prepare (encode) ─────────────────────────────────────────────────────
  setStateFn({ phase: "preparing", error: null });

  let fileBytes: number[];
  let fileName: string;
  try {
    const encoded = await encodeFn();
    fileBytes = encoded.fileBytes;
    fileName = encoded.fileName;
  } catch (err) {
    setStateFn({
      phase: "error",
      error:
        err instanceof Error
          ? `Encode failed: ${err.message}`
          : "Encode failed.",
    });
    return false;
  }

  // ── Eligibility checkpoint 2: after encode, before upload ─────────────────
  // State can change while encode is awaited (channel archived, membership
  // lost, timeout received, relay-self resolves to classify DM).
  const reason2 = checkEligibilityFn();
  if (reason2 !== null) {
    setStateFn({ phase: "error", error: reason2 });
    return false;
  }

  // ── Upload ────────────────────────────────────────────────────────────────
  setStateFn({ phase: "uploading", error: null });

  let descriptor: BlobDescriptor;
  try {
    descriptor = await uploadFn(fileBytes, fileName);
  } catch (err) {
    setStateFn({
      phase: "error",
      error:
        err instanceof Error
          ? `Upload failed: ${err.message}`
          : "Upload failed.",
    });
    return false;
  }

  // Preserve the original filename so `buildImetaTags` emits a `filename`
  // field and the recipient's FileCard renders the correct label. Snapshot
  // sends never emit `thumb`: NIP-92 requires it to be this upload's local
  // thumbnail sidecar, which an agent avatar is not.
  const { thumb: _thumb, ...descriptorWithoutThumb } = descriptor;
  const descriptorWithFilename: BlobDescriptor = {
    ...descriptorWithoutThumb,
    filename: fileName,
  };

  // ── Build message content + NIP-92 imeta tags ─────────────────────────────
  const { content, mediaTags } = buildMessageFn(descriptorWithFilename);

  // ── Send ──────────────────────────────────────────────────────────────────
  setStateFn({ phase: "sending", error: null });

  try {
    await sendFn({
      channelId,
      content,
      mediaTags: mediaTags ?? [],
    });
  } catch (err) {
    setStateFn({
      phase: "error",
      error:
        err instanceof Error ? `Send failed: ${err.message}` : "Send failed.",
    });
    return false;
  }

  setStateFn({ phase: "done", error: null });
  return true;
}

/**
 * Compose the single-concurrency guard with the send pipeline.
 *
 * This is the exact production composition that `beginSend` uses: a second
 * concurrent call to `runGuardedSend` with the same guard receives `false`
 * immediately — encode never starts for the blocked call.
 *
 * Exported so unit tests can import and call this exact function twice
 * concurrently with injected counters, proving one encode/upload/send and one
 * blocked result.  A test that stays green after encode is moved outside the
 * guard is not a production-composition test; this function is.
 */
export function runGuardedSend(
  guard: ReturnType<typeof createSendGuard>,
  pipelineDeps: Parameters<typeof runSendPipeline>[0],
): Promise<boolean> {
  return guard.runGuarded(() => runSendPipeline(pipelineDeps));
}

export type UseSnapshotSendControllerResult = {
  /**
   * Sendable destinations with resolved display labels.  DMs are omitted
   * while identity or relay-self are loading (fail-closed moderation-DM race).
   */
  sendableChannels: ResolvedChannel[];
  /** True while channels, identity, or relay-self are loading. */
  isLoadingChannels: boolean;
  state: SnapshotSendState;
  /**
   * Execute the full prepare → encode → upload → send sequence behind a
   * single-concurrency guard.  A second call while the first is in-flight
   * returns `false` immediately — encode never starts for the blocked call.
   *
   * Eligibility is checked at two internal checkpoints by reading directly
   * from the React Query cache and the timeout external store — not from
   * rendered React state or component refs.  This closes both the
   * pre-flight/guard-entry race and the during-encode state-change race.
   *
   * The caller MUST have already obtained explicit destination-scoped
   * confirmation for memory-bearing payloads before calling this.  Returns
   * false and sets error state if blocked, ineligible, or if any step fails.
   * Never throws.
   *
   */
  beginSend: (
    encodeFn: () => Promise<{ fileBytes: number[]; fileName: string }>,
    channelId: string,
  ) => Promise<boolean>;
  /** Set state to error with a message (for pre-send gate failures). */
  setErrorState: (message: string) => void;
  reset: () => void;
};

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useSnapshotSendController(): UseSnapshotSendControllerResult {
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const queryClient = useQueryClient();

  // Only fetch relay self when there are DM candidates — same gate as ChannelPane.
  const hasDmCandidates = React.useMemo(
    () =>
      (channelsQuery.data ?? []).some(
        (ch) => ch.channelType === "dm" && isSendableDestination(ch),
      ),
    [channelsQuery.data],
  );
  const relaySelfQuery = useRelaySelfQuery(hasDmCandidates);

  // Collect the "other participant" pubkeys from all DM candidates so we can
  // resolve their display names.  Kept stable by memo so the batch query key
  // doesn't flap on every render.
  const dmParticipantPubkeys = React.useMemo(() => {
    const currentPubkey = identityQuery.data?.pubkey?.toLowerCase();
    return (channelsQuery.data ?? [])
      .filter((ch) => ch.channelType === "dm" && isSendableDestination(ch))
      .flatMap((ch) =>
        ch.participantPubkeys.filter(
          (pk) => pk.toLowerCase() !== currentPubkey,
        ),
      );
  }, [channelsQuery.data, identityQuery.data]);

  const dmProfilesQuery = useUsersBatchQuery(dmParticipantPubkeys, {
    enabled: dmParticipantPubkeys.length > 0,
  });

  const [state, setState] = React.useState<SnapshotSendState>({
    phase: "idle",
    error: null,
  });

  // Single-concurrency guard covering the full encode → upload → send action.
  // Stored in a ref so it survives re-renders without triggering effects.
  const guardRef = React.useRef(createSendGuard());

  // Pass null channel here — we supply the captured channelId per-send instead.
  const sendMutation = useSendMessageMutation(null, identityQuery.data);

  const sendableChannels = React.useMemo<ResolvedChannel[]>(() => {
    const currentPubkey = identityQuery.data?.pubkey;
    const relaySelf = relaySelfQuery.data;
    // Fail-closed: withhold ALL DMs until BOTH identity AND relay-self have
    // successfully resolved (`status === "success"`).  Absent, pending,
    // fetching, and errored states are all unknown — we cannot classify
    // whether a DM is a moderation DM without valid identity + relay-self.
    // A successfully resolved `relaySelf === null` IS known: the relay
    // advertises no self, and the moderation helper's fail-open applies.
    const identitySuccess = identityQuery.status === "success";
    const relaySelfSuccess = relaySelfQuery.status === "success";
    const dmGateOpen =
      !hasDmCandidates || (identitySuccess && relaySelfSuccess);
    const dmProfiles = dmProfilesQuery.data?.profiles;

    return (channelsQuery.data ?? [])
      .filter(
        (ch) =>
          isSendableDestination(ch) &&
          !isModerationDm(ch, currentPubkey, relaySelf) &&
          (ch.channelType !== "dm" || dmGateOpen),
      )
      .map((ch) => ({
        ...ch,
        displayLabel: resolveChannelDisplayLabel(ch, currentPubkey, dmProfiles),
      }));
  }, [
    channelsQuery.data,
    identityQuery.data,
    identityQuery.status,
    relaySelfQuery.data,
    relaySelfQuery.status,
    hasDmCandidates,
    dmProfilesQuery.data,
  ]);

  async function beginSend(
    encodeFn: () => Promise<{ fileBytes: number[]; fileName: string }>,
    channelId: string,
  ): Promise<boolean> {
    return runGuardedSend(guardRef.current, {
      encodeFn,
      channelId,
      // Eligibility is checked by reading directly from the React Query cache
      // and the timeout external store — not from rendered React state.
      checkEligibilityFn: () => checkSendEligibility(queryClient, channelId),
      uploadFn: (bytes, filename) => uploadMediaBytes(bytes, filename),
      sendFn: (args) => sendMutation.mutateAsync(args),
      setStateFn: setState,
      buildMessageFn: (descriptor) => buildOutgoingMessage("", [descriptor]),
    });
  }

  function reset() {
    if (!guardRef.current.inFlight) {
      setState({ phase: "idle", error: null });
    }
  }

  return {
    sendableChannels,
    isLoadingChannels:
      channelsQuery.isLoading ||
      (hasDmCandidates &&
        (relaySelfQuery.isLoading || identityQuery.isLoading)),
    state,
    beginSend,
    setErrorState: (message: string) => {
      setState({ phase: "error", error: message });
    },
    reset,
  };
}
