/**
 * useSnapshotSendController
 *
 * Payload-agnostic upload → send controller for sharing a snapshot to a Buzz
 * channel or DM.  The caller supplies pre-encoded bytes + filename from the
 * Rust layer; this hook drives uploadMediaBytes → sendChannelMessage with
 * honest progress and idempotent double-send protection.
 *
 * This hook does not know what kind of snapshot the bytes contain.  A future
 * team-snapshot or other payload can reuse it unchanged by passing different
 * bytes and a filename.  Hard-coded semantics for `.agent.*` live only in
 * the export-dialog layer above this hook.
 */

import * as React from "react";

import {
  uploadMediaBytes,
  sendChannelMessage,
  type BlobDescriptor,
} from "@/shared/api/tauri";
import { buildOutgoingMessage } from "@/features/messages/lib/imetaMediaMarkdown";
import { useChannelsQuery } from "@/features/channels/hooks";
import type { Channel } from "@/shared/api/types";

// ── Public types ──────────────────────────────────────────────────────────────

export type SendPhase = "idle" | "uploading" | "sending" | "done" | "error";

export type SnapshotSendState = {
  phase: SendPhase;
  error: string | null;
};

/**
 * A joined, non-archived, sendable destination: channelType "stream" or "dm",
 * isMember true, archivedAt null.  Mirrors the canSendDraft gate exactly.
 */
export function isSendableDestination(ch: Channel): boolean {
  return ch.isMember && ch.archivedAt === null && ch.channelType !== "forum";
}

export type UseSnapshotSendControllerResult = {
  /** Sendable destinations the user may select (loaded from the channel cache). */
  sendableChannels: Channel[];
  /** True while the channels query is loading for the first time. */
  isLoadingChannels: boolean;
  state: SnapshotSendState;
  /**
   * Upload `bytes` and send them to `channelId` as a standard NIP-92 imeta
   * attachment message.
   *
   * The caller MUST have already obtained explicit destination-scoped
   * confirmation for memory-bearing payloads before calling this.  Returns
   * false and sets error state if a send is already in progress (double-send
   * guard) or if any step fails.  The method never throws.
   */
  sendPayload: (
    bytes: number[],
    filename: string,
    channelId: string,
  ) => Promise<boolean>;
  reset: () => void;
};

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useSnapshotSendController(): UseSnapshotSendControllerResult {
  const channelsQuery = useChannelsQuery();
  const [state, setState] = React.useState<SnapshotSendState>({
    phase: "idle",
    error: null,
  });

  // Prevent double-send between renders.
  const inFlightRef = React.useRef(false);

  const sendableChannels = React.useMemo(
    () => (channelsQuery.data ?? []).filter(isSendableDestination),
    [channelsQuery.data],
  );

  async function sendPayload(
    bytes: number[],
    filename: string,
    channelId: string,
  ): Promise<boolean> {
    if (inFlightRef.current) return false;
    inFlightRef.current = true;

    try {
      // ── Upload ────────────────────────────────────────────────────────────
      setState({ phase: "uploading", error: null });

      let descriptor: BlobDescriptor;
      try {
        descriptor = await uploadMediaBytes(bytes, filename);
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
        filename,
      };

      // ── Build message content + NIP-92 imeta tags ─────────────────────────
      const { content, mediaTags } = buildOutgoingMessage("", [
        descriptorWithFilename,
      ]);

      // ── Send to the captured destination ──────────────────────────────────
      setState({ phase: "sending", error: null });

      try {
        await sendChannelMessage(channelId, content, null, mediaTags ?? []);
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
    } finally {
      inFlightRef.current = false;
    }
  }

  function reset() {
    if (!inFlightRef.current) {
      setState({ phase: "idle", error: null });
    }
  }

  return {
    sendableChannels,
    isLoadingChannels: channelsQuery.isLoading,
    state,
    sendPayload,
    reset,
  };
}
