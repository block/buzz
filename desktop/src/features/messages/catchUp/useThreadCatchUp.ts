import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type { TimelineMessage } from "@/features/messages/types";
import type { Channel } from "@/shared/api/types";
import {
  type CatchUpRecap,
  type CatchUpScope,
  type CatchUpSettings,
  cancelThreadCatchUp,
  correctThreadCatchUp,
  generateThreadCatchUp,
  getThreadCatchUpSettings,
  listThreadCatchUpArtifacts,
  type SharedCatchUpList,
  setThreadCatchUpSettings,
  shareThreadCatchUp,
} from "@/shared/api/tauriThreadCatchUp";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import {
  type CatchUpBaseline,
  type CatchUpRangeChoice,
  resolveCatchUpBaseline,
  resolveCatchUpRange,
  snapshotCitedBodies,
} from "./threadCatchUp";
import { upsertSharedCatchUp } from "./threadCatchUpArtifact";
import { useThreadCatchUpBaseline } from "./useThreadCatchUpBaseline";
import {
  EMPTY_CATCH_UP_READ_HANDOFF,
  type ThreadCatchUpReadHandoff,
} from "./useThreadCatchUpReadHandoff";

const HEX64_RE = /^[a-f0-9]{64}$/i;
export const catchUpSettingsQueryKey = ["thread-catch-up", "settings"] as const;
export function catchUpArtifactsQueryKey(scopeKey: string) {
  return ["thread-catch-up", "artifacts", scopeKey] as const;
}

export type CatchUpPhase =
  | { kind: "idle" }
  | { kind: "generating"; requestId: string; choice: CatchUpRangeChoice }
  | {
      kind: "ready";
      recap: CatchUpRecap;
      choice: CatchUpRangeChoice;
      /** Cited bodies as loaded when the recap arrived, to flag later edits. */
      citedBodies: ReadonlyMap<string, string>;
    }
  | { kind: "error"; message: string; choice: CatchUpRangeChoice };

function errorMessage(error: unknown) {
  return error instanceof Error && error.message
    ? error.message
    : "Something went wrong.";
}

/**
 * Whether a thread can offer a recap: a standard channel thread whose root is
 * an ordinary message, read by a member in a known community and identity.
 * Huddle transcripts, forum posts, and DMs are excluded.
 */
export function isThreadCatchUpEligible({
  channel,
  isHuddleTranscript,
  threadHead,
  relayUrl,
  currentPubkey,
}: {
  channel: Pick<Channel, "channelType" | "isMember"> | null;
  isHuddleTranscript: boolean;
  threadHead: Pick<TimelineMessage, "id" | "kind"> | null;
  relayUrl?: string | null;
  currentPubkey?: string | null;
}) {
  return Boolean(
    channel &&
      channel.channelType === "stream" &&
      channel.isMember &&
      !isHuddleTranscript &&
      threadHead &&
      HEX64_RE.test(threadHead.id) &&
      (threadHead.kind === KIND_STREAM_MESSAGE ||
        threadHead.kind === KIND_STREAM_MESSAGE_V2) &&
      relayUrl?.trim() &&
      currentPubkey &&
      HEX64_RE.test(currentPubkey),
  );
}

export type ThreadCatchUpController = ReturnType<
  typeof useThreadCatchUpController
>;

/**
 * Channel-level owner of one thread's recap. Lives above the thread panel so
 * a recap survives navigating to its sources, and resets when the captured
 * relay, identity, channel, or thread changes. Generating never posts, marks
 * anything read, or changes tasks; sharing is a separate explicit action.
 */
export function useThreadCatchUpController({
  channel,
  isHuddleTranscript,
  threadHead,
  threadMessages,
  relayUrl,
  currentPubkey,
  getOwnReadAt,
  isReadStateReady,
  readHandoff = EMPTY_CATCH_UP_READ_HANDOFF,
}: {
  channel: Channel | null;
  isHuddleTranscript: boolean;
  threadHead: TimelineMessage | null;
  threadMessages: readonly TimelineMessage[];
  relayUrl?: string | null;
  currentPubkey?: string | null;
  getOwnReadAt: (contextId: string) => number | null;
  isReadStateReady: boolean;
  readHandoff?: ThreadCatchUpReadHandoff;
}) {
  const queryClient = useQueryClient();
  const eligible = isThreadCatchUpEligible({
    channel,
    isHuddleTranscript,
    threadHead,
    relayUrl,
    currentPubkey,
  });
  const rootEventId =
    eligible && threadHead ? threadHead.id.toLowerCase() : null;
  const threadMessageIds = React.useMemo(
    () => threadMessages.map((message) => message.id.toLowerCase()),
    [threadMessages],
  );
  const pendingRead = channel ? readHandoff.peek(channel.id) : null;
  const snapshot = useThreadCatchUpBaseline({
    channelId: channel?.id ?? null,
    rootEventId,
    threadMessageIds,
    getOwnReadAt,
    isReadStateReady,
    inboxSnapshot: pendingRead?.snapshot,
    scopeKey: JSON.stringify([relayUrl, currentPubkey?.toLowerCase()]),
  });
  React.useEffect(() => {
    if (pendingRead) readHandoff.consume(pendingRead.key);
  }, [pendingRead, readHandoff]);

  const scope = React.useMemo<CatchUpScope | null>(
    () =>
      rootEventId && channel && relayUrl && currentPubkey
        ? {
            relayUrl,
            signerPubkey: currentPubkey.toLowerCase(),
            channelId: channel.id,
            rootEventId,
          }
        : null,
    [channel, currentPubkey, relayUrl, rootEventId],
  );
  const scopeKey = scope
    ? [
        scope.relayUrl,
        scope.signerPubkey,
        scope.channelId,
        scope.rootEventId,
      ].join("|")
    : null;

  const [isOpen, setIsOpen] = React.useState(false);
  const [phase, setPhase] = React.useState<CatchUpPhase>({ kind: "idle" });
  // The thread this controller last served. A thread that is momentarily
  // unloaded (for example while navigating to a cited source) keeps its
  // recap; a different relay, identity, channel, or thread discards it.
  const [servedScopeKey, setServedScopeKey] = React.useState(scopeKey);
  if (scopeKey && scopeKey !== servedScopeKey) {
    setServedScopeKey(scopeKey);
    setPhase({ kind: "idle" });
    setIsOpen(false);
  }
  // The one request whose result may still land. Cleared on cancel, on
  // completion, and whenever the served thread changes, so late results from
  // another scope are dropped.
  const inFlightRef = React.useRef<string | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: the cleanup must run when the served thread changes
  React.useEffect(
    () => () => {
      const requestId = inFlightRef.current;
      inFlightRef.current = null;
      if (requestId) void cancelThreadCatchUp(requestId).catch(() => {});
    },
    [servedScopeKey],
  );

  const bodies = React.useMemo(() => {
    const map = new Map<string, string>();
    for (const message of threadMessages)
      map.set(message.id.toLowerCase(), message.body);
    return map;
  }, [threadMessages]);
  const bodiesRef = React.useRef(bodies);
  bodiesRef.current = bodies;

  const nowSeconds = Math.floor(Date.now() / 1_000);
  const baseline: CatchUpBaseline = resolveCatchUpBaseline({
    snapshot,
    replies: threadMessages.map((message) => ({
      id: message.id.toLowerCase(),
      createdAt: message.createdAt,
      pubkey: message.pubkey,
    })),
    currentPubkey: currentPubkey ?? undefined,
    nowSeconds,
  });
  const baselineRef = React.useRef(baseline);
  baselineRef.current = baseline;

  const settingsQuery = useQuery({
    queryKey: catchUpSettingsQueryKey,
    queryFn: getThreadCatchUpSettings,
    enabled: eligible && isOpen,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
  const saveSettings = useMutation({
    mutationFn: (input: CatchUpSettings) => setThreadCatchUpSettings(input),
    onSuccess: (saved) => {
      queryClient.setQueryData(catchUpSettingsQueryKey, saved);
    },
  });

  const artifactsKey = catchUpArtifactsQueryKey(scopeKey ?? "none");
  const artifactsQuery = useQuery({
    queryKey: artifactsKey,
    queryFn: () => {
      if (!scope) throw new Error("No thread selected.");
      return listThreadCatchUpArtifacts(scope);
    },
    enabled: Boolean(scope) && isOpen,
    staleTime: 30_000,
  });

  const generate = React.useCallback(
    (choice: CatchUpRangeChoice) => {
      if (!scope) return;
      const range = resolveCatchUpRange(
        choice,
        baselineRef.current,
        Math.floor(Date.now() / 1_000),
      );
      if (!range) return;
      const previous = inFlightRef.current;
      if (previous) void cancelThreadCatchUp(previous).catch(() => {});
      const requestId = crypto.randomUUID();
      inFlightRef.current = requestId;
      setPhase({ kind: "generating", requestId, choice });
      const settle = (next: CatchUpPhase) => {
        if (inFlightRef.current !== requestId) return;
        inFlightRef.current = null;
        setPhase(next);
      };
      generateThreadCatchUp({ requestId, scope, ...range }).then(
        (recap) => {
          if (
            recap.rootEventId !== scope.rootEventId ||
            recap.channelId !== scope.channelId
          ) {
            settle({
              kind: "error",
              message: "The recap did not match this thread.",
              choice,
            });
            return;
          }
          settle({
            kind: "ready",
            recap,
            choice,
            citedBodies: snapshotCitedBodies(recap, bodiesRef.current),
          });
        },
        (error: unknown) => {
          settle({ kind: "error", message: errorMessage(error), choice });
        },
      );
    },
    [scope],
  );

  const cancel = React.useCallback(() => {
    const requestId = inFlightRef.current;
    inFlightRef.current = null;
    if (requestId) void cancelThreadCatchUp(requestId).catch(() => {});
    setPhase({ kind: "idle" });
  }, []);

  const share = useMutation({
    mutationFn: (recap: CatchUpRecap) => {
      if (!scope) throw new Error("No thread selected.");
      return shareThreadCatchUp({ scope, recap });
    },
    onSuccess: (shared) => {
      queryClient.setQueryData<SharedCatchUpList>(artifactsKey, (list) =>
        upsertSharedCatchUp(list, shared),
      );
    },
  });

  const correct = useMutation({
    mutationFn: (input: {
      artifactId: string;
      prevEventId: string;
      pointId: string;
      text: string;
    }) => {
      if (!scope) throw new Error("No thread selected.");
      return correctThreadCatchUp({ scope, ...input, text: input.text.trim() });
    },
    onSuccess: (shared) => {
      queryClient.setQueryData<SharedCatchUpList>(artifactsKey, (list) =>
        upsertSharedCatchUp(list, shared),
      );
    },
  });

  const refreshShared = React.useCallback(
    () =>
      queryClient.invalidateQueries({
        queryKey: catchUpArtifactsQueryKey(scopeKey ?? "none"),
      }),
    [queryClient, scopeKey],
  );

  if (!eligible || !scope || !scopeKey) return null;
  return {
    scope,
    scopeKey,
    isOpen,
    setIsOpen,
    phase,
    baseline,
    generate,
    cancel,
    settingsQuery,
    saveSettings,
    artifactsQuery,
    refreshShared,
    share,
    correct,
    threadMessages,
    bodies,
    isArchived: Boolean(channel?.archivedAt),
  };
}
