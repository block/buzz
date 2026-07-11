/**
 * useSnapshotSendController
 *
 * Payload-agnostic upload → send controller for sharing a snapshot to a Buzz
 * channel or DM.  The caller supplies an encode function and a destination;
 * the controller drives prepare → encode → upload → send with honest progress,
 * idempotent double-send protection covering the entire action (not just upload),
 * and fail-closed moderation-DM race handling.
 *
 * This hook does not know what kind of snapshot the bytes contain.  A future
 * team-snapshot or other payload can reuse it unchanged by passing different
 * bytes and a filename.  Hard-coded semantics for `.agent.*` live only in
 * the export-dialog layer above this hook.
 */

import * as React from "react";

import { uploadMediaBytes, type BlobDescriptor } from "@/shared/api/tauri";
import { buildOutgoingMessage } from "@/features/messages/lib/imetaMediaMarkdown";
import { useChannelsQuery } from "@/features/channels/hooks";
import { isModerationDm } from "@/features/moderation/lib/moderationDm";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import type { Channel } from "@/shared/api/types";

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
   * `encodeFn` is called after the guard is acquired and the `preparing` phase
   * is set; its result feeds directly into the upload step so the guard covers
   * the entire action including memory fetch/encode.
   *
   * The caller MUST have already obtained explicit destination-scoped
   * confirmation for memory-bearing payloads before calling this.  Returns
   * false and sets error state if blocked or if any step fails.  Never throws.
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
    // Fail-closed: withhold ALL DMs until BOTH identity AND relay-self are
    // known.  Identity is required so isModerationDm can compare the current
    // user's pubkey against the DM participant list.  If relay-self resolves
    // before identity, isModerationDm receives no currentPubkey, treats both
    // participants as "others," and can fail to classify a 1:1 moderation DM.
    const dmGateOpen =
      !hasDmCandidates ||
      (!relaySelfQuery.isLoading && !identityQuery.isLoading);
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
    identityQuery.isLoading,
    relaySelfQuery.data,
    relaySelfQuery.isLoading,
    hasDmCandidates,
    dmProfilesQuery.data,
  ]);

  async function beginSend(
    encodeFn: () => Promise<{ fileBytes: number[]; fileName: string }>,
    channelId: string,
  ): Promise<boolean> {
    return guardRef.current.runGuarded(async () => {
      // ── Prepare (encode) ─────────────────────────────────────────────────
      setState({ phase: "preparing", error: null });

      let fileBytes: number[];
      let fileName: string;
      try {
        const encoded = await encodeFn();
        fileBytes = encoded.fileBytes;
        fileName = encoded.fileName;
      } catch (err) {
        setState({
          phase: "error",
          error:
            err instanceof Error
              ? `Encode failed: ${err.message}`
              : "Encode failed.",
        });
        return false;
      }

      // ── Upload ────────────────────────────────────────────────────────────
      setState({ phase: "uploading", error: null });

      let descriptor: BlobDescriptor;
      try {
        descriptor = await uploadMediaBytes(fileBytes, fileName);
      } catch (err) {
        setState({
          phase: "error",
          error:
            err instanceof Error
              ? `Upload failed: ${err.message}`
              : "Upload failed.",
        });
        return false;
      }

      // Preserve the original filename in the descriptor so `buildImetaTags`
      // emits a `filename` field and the recipient's FileCard renders the
      // correct label.
      const descriptorWithFilename: BlobDescriptor = {
        ...descriptor,
        filename: fileName,
      };

      // ── Build message content + NIP-92 imeta tags ─────────────────────────
      const { content, mediaTags } = buildOutgoingMessage("", [
        descriptorWithFilename,
      ]);

      // ── Send to the captured destination via canonical mutation ────────────
      // Capturing channelId here prevents a channel switch from redirecting
      // the attachment; useSendMessageMutation resolves the live channel from
      // the query cache using the supplied id.
      setState({ phase: "sending", error: null });

      try {
        await sendMutation.mutateAsync({
          channelId,
          content,
          mediaTags: mediaTags ?? [],
        });
      } catch (err) {
        setState({
          phase: "error",
          error:
            err instanceof Error
              ? `Send failed: ${err.message}`
              : "Send failed.",
        });
        return false;
      }

      setState({ phase: "done", error: null });
      return true;
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
