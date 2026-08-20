import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useActiveAgentTurnsByChannel } from "@/features/agents/activeAgentTurnsStore";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  useChannelMessagesQuery,
  useSendMessageMutation,
} from "@/features/messages/hooks";
import {
  parseBoardReferences,
  boardReferenceKey,
  resolveBoardReferences,
  type BoardReference,
} from "@/features/workstream-board/lib/boardReferences";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { filterWorkstreamChannels } from "@/features/workstream-board/lib/discoverWorkstreamChannels";
import { getActiveWorkstreamTurns } from "@/features/workstream-board/lib/activeWorkstreamTurns";
import {
  sortWorkstreamCards,
  type WorkstreamWait,
} from "@/features/workstream-board/lib/workstreamWaits";
import { WorkstreamCard } from "@/features/workstream-board/ui/WorkstreamCard";
import { BOARD_REPLAY_STORAGE_KEY } from "@/features/workstream-board/ui/BoardReferenceSet";
import { BoardReferenceItems } from "@/features/workstream-board/ui/BoardReferenceItems";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { PageHeader } from "@/shared/ui/PageHeader";

const WORKSTREAM_CARD_GRID_CLASS =
  "grid grid-cols-1 gap-3 [@container(min-width:38rem)]:grid-cols-2 [@container(min-width:54rem)]:grid-cols-3";
type CardMetadata = {
  waits: readonly WorkstreamWait[];
  createdAt: string | null | undefined;
};

function sameMetadata(
  left: CardMetadata | undefined,
  right: CardMetadata,
): boolean {
  return (
    left?.createdAt === right.createdAt &&
    JSON.stringify(left?.waits) === JSON.stringify(right.waits)
  );
}

export function WorkstreamBoardScreen() {
  const { goChannel } = useAppNavigation();
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const [discussionMode, setDiscussionMode] = React.useState(false);
  const [selectedReferences, setSelectedReferences] = React.useState<
    BoardReference[]
  >([]);
  const discussionChannelId =
    selectedReferences[0]?.placement.workstreamId ?? null;
  const [draft, setDraft] = React.useState("");
  const [replayReferences, setReplayReferences] = React.useState<
    BoardReference[]
  >([]);
  const [replayFocusIndex, setReplayFocusIndex] = React.useState(0);
  const startReplay = React.useCallback((references: BoardReference[]) => {
    setDiscussionMode(true);
    setReplayFocusIndex(0);
    setReplayReferences(references);
  }, []);
  React.useEffect(() => {
    if (!discussionMode && replayReferences.length === 0) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (replayReferences.length > 0) setReplayReferences([]);
        else if (
          (selectedReferences.length === 0 && draft.length === 0) ||
          window.confirm(
            "Discard the unsent discussion draft and reference tray?",
          )
        ) {
          setDiscussionMode(false);
          setSelectedReferences([]);
          setDraft("");
        }
      } else if (
        replayReferences.length > 0 &&
        (event.key === "ArrowRight" || event.key === "]")
      ) {
        event.preventDefault();
        setReplayFocusIndex(
          (current) => (current + 1) % replayReferences.length,
        );
      } else if (
        replayReferences.length > 0 &&
        (event.key === "ArrowLeft" || event.key === "[")
      ) {
        event.preventDefault();
        setReplayFocusIndex(
          (current) =>
            (current - 1 + replayReferences.length) % replayReferences.length,
        );
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [discussionMode, draft, replayReferences, selectedReferences.length]);
  const [liveReferencesByChannel, setLiveReferencesByChannel] = React.useState<
    ReadonlyMap<string, readonly BoardReference[]>
  >(() => new Map());
  React.useEffect(() => {
    const stored = sessionStorage.getItem(BOARD_REPLAY_STORAGE_KEY);
    if (!stored) return;
    sessionStorage.removeItem(BOARD_REPLAY_STORAGE_KEY);
    try {
      const parsed: unknown = JSON.parse(stored);
      if (Array.isArray(parsed)) {
        startReplay(
          parsed.flatMap((value) => {
            const references = parseBoardReferences([
              ["buzz:board-ref", "1", JSON.stringify(value)],
            ]);
            return references;
          }),
        );
      }
    } catch {
      // Malformed cross-route replay state fails closed.
    }
  }, [startReplay]);
  const workstreamChannels = filterWorkstreamChannels(channelsQuery.data ?? []);
  const discussionChannel =
    workstreamChannels.find((channel) => channel.id === discussionChannelId) ??
    null;
  const discussionMessages = useChannelMessagesQuery(discussionChannel);
  const sendMessage = useSendMessageMutation(
    discussionChannel,
    identityQuery.data,
  );
  const selectedKeys = React.useMemo(
    () => new Set(selectedReferences.map(boardReferenceKey)),
    [selectedReferences],
  );
  const replayKeys = React.useMemo(
    () => new Set(replayReferences.map(boardReferenceKey)),
    [replayReferences],
  );
  const replayFocusedKey = replayReferences[replayFocusIndex]
    ? boardReferenceKey(replayReferences[replayFocusIndex])
    : undefined;
  const liveReferences = React.useMemo(
    () =>
      new Map(
        [...liveReferencesByChannel.values()]
          .flat()
          .map((reference) => [boardReferenceKey(reference), reference]),
      ),
    [liveReferencesByChannel],
  );
  const resolvedReplayReferences = React.useMemo(
    () => resolveBoardReferences(replayReferences, liveReferences),
    [liveReferences, replayReferences],
  );
  const replayReferenceStates = React.useMemo(
    () =>
      new Map(
        resolvedReplayReferences.flatMap((item) =>
          item.state === "historical"
            ? []
            : [[boardReferenceKey(item.reference), item.state] as const],
        ),
      ),
    [resolvedReplayReferences],
  );
  const onReferencesChange = React.useCallback(
    (channelId: string, references: readonly BoardReference[]) => {
      setLiveReferencesByChannel((current) => {
        const previous = current.get(channelId);
        if (JSON.stringify(previous) === JSON.stringify(references))
          return current;
        const next = new Map(current);
        next.set(channelId, references);
        return next;
      });
    },
    [],
  );
  const toggleReference = React.useCallback((reference: BoardReference) => {
    setSelectedReferences((current) => {
      if (
        current.length > 0 &&
        current[0].placement.workstreamId !== reference.placement.workstreamId
      )
        return current;
      const key = boardReferenceKey(reference);
      return current.some((item) => boardReferenceKey(item) === key)
        ? current.filter((item) => boardReferenceKey(item) !== key)
        : [...current, reference];
    });
  }, []);
  const activeTurnsByChannel = useActiveAgentTurnsByChannel();
  const activeWorkstreamTurns = React.useMemo(
    () => getActiveWorkstreamTurns(workstreamChannels, activeTurnsByChannel),
    [activeTurnsByChannel, workstreamChannels],
  );
  const activeTurnsByChannelId = React.useMemo(
    () => new Map(activeWorkstreamTurns.map((turn) => [turn.channelId, turn])),
    [activeWorkstreamTurns],
  );
  const activeAgentPubkeys = React.useMemo(
    () => activeWorkstreamTurns.flatMap((turn) => turn.agentPubkeys),
    [activeWorkstreamTurns],
  );
  const activeProfilesQuery = useUsersBatchQuery(activeAgentPubkeys);
  const [metadataByChannel, setMetadataByChannel] = React.useState<
    ReadonlyMap<string, CardMetadata>
  >(() => new Map());
  const onWaitsChange = React.useCallback(
    (
      channelId: string,
      waits: readonly WorkstreamWait[],
      createdAt: string | null | undefined,
    ) => {
      const next = { waits, createdAt };
      setMetadataByChannel((current) => {
        if (sameMetadata(current.get(channelId), next)) return current;
        const updated = new Map(current);
        updated.set(channelId, next);
        return updated;
      });
    },
    [],
  );
  const sortedChannels = React.useMemo(
    () =>
      sortWorkstreamCards(
        workstreamChannels.map((channel) => ({
          channelId: channel.id,
          channelName: channel.name,
          createdAt: metadataByChannel.get(channel.id)?.createdAt,
          waits: metadataByChannel.get(channel.id)?.waits ?? [],
          activeWorking: activeTurnsByChannelId.get(channel.id),
          channel,
        })),
        identityQuery.data?.pubkey,
      ).map((entry) => entry.channel),
    [
      activeTurnsByChannelId,
      identityQuery.data?.pubkey,
      metadataByChannel,
      workstreamChannels,
    ],
  );

  return (
    <div
      className="relative flex min-h-0 flex-1 overflow-hidden"
      data-testid="workstream-board-view"
    >
      <div
        className="flex min-h-0 flex-1 flex-col overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8"
        data-scroll-restoration-id="workstream-board-list"
      >
        <div className="mx-auto w-full max-w-6xl space-y-8 [container-type:inline-size]">
          <PageHeader
            description="Live canvases for active workstream channels."
            title="Workstream Board"
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button
              onClick={() => {
                if (
                  discussionMode &&
                  (selectedReferences.length > 0 || draft.length > 0) &&
                  !window.confirm(
                    "Discard the unsent discussion draft and reference tray?",
                  )
                )
                  return;
                setDiscussionMode(!discussionMode);
                setSelectedReferences([]);
                setReplayReferences([]);
                setDraft("");
              }}
              variant={discussionMode ? "secondary" : "default"}
            >
              {discussionMode ? "Exit discussion" : "Discuss Board"}
            </Button>
            {replayReferences.length > 0 ? (
              <Button onClick={() => setReplayReferences([])} variant="outline">
                End replay
              </Button>
            ) : null}
          </div>
          {discussionMode ? (
            <section
              aria-label="Board discussion"
              className="space-y-3 rounded-xl border bg-muted/30 p-4"
              data-testid="board-discussion"
            >
              <p className="text-sm font-semibold">
                Reference tray ({selectedReferences.length})
              </p>
              <BoardReferenceItems
                onRemove={toggleReference}
                references={selectedReferences}
              />
              <textarea
                aria-label="Discussion message"
                className="min-h-20 w-full rounded-md border bg-background p-2 text-sm"
                onChange={(event) => setDraft(event.target.value)}
                placeholder="Discuss these Board references…"
                value={draft}
              />
              <Button
                disabled={
                  !discussionChannel ||
                  (!draft.trim() && selectedReferences.length === 0) ||
                  sendMessage.isPending
                }
                onClick={async () => {
                  if (!discussionChannel) return;
                  await sendMessage.mutateAsync({
                    content: draft || "Board references",
                    boardReferences: selectedReferences,
                  });
                  setDraft("");
                  setSelectedReferences([]);
                }}
              >
                Send
              </Button>
              {discussionChannel ? (
                <div className="space-y-2 border-t pt-3">
                  <p className="text-xs font-semibold">
                    #{discussionChannel.name} messages
                  </p>
                  {(discussionMessages.data ?? [])
                    .filter(
                      (event) =>
                        parseBoardReferences(event.tags, discussionChannel.id)
                          .length > 0,
                    )
                    .map((event) => {
                      const refs = parseBoardReferences(
                        event.tags,
                        discussionChannel.id,
                      );
                      return (
                        <button
                          className="block w-full rounded-md border bg-background p-2 text-left text-xs"
                          key={event.id}
                          onClick={() => startReplay(refs)}
                          type="button"
                        >
                          <span className="block">{event.content}</span>
                          <BoardReferenceItems references={refs} />
                        </button>
                      );
                    })}
                </div>
              ) : (
                <p className="text-xs text-muted-foreground">
                  Select an object in one workstream to choose the discussion.
                </p>
              )}
              {replayReferences.length > 0
                ? (() => {
                    const resolved = resolvedReplayReferences;
                    const fallbackHistorical = resolved.filter(
                      (item) =>
                        item.state === "historical" &&
                        !workstreamChannels.some(
                          (channel) =>
                            channel.id ===
                            item.reference.placement.workstreamId,
                        ),
                    );
                    return (
                      <div className="text-xs">
                        <strong>
                          Replay {replayFocusIndex + 1}/
                          {replayReferences.length}:
                        </strong>{" "}
                        {
                          resolved.filter((item) => item.state === "live")
                            .length
                        }{" "}
                        live ·{" "}
                        {
                          resolved.filter((item) => item.state === "changed")
                            .length
                        }{" "}
                        changed ·{" "}
                        {
                          resolved.filter((item) => item.state === "historical")
                            .length
                        }{" "}
                        historical
                        <span className="ml-2 inline-flex gap-1">
                          <button
                            aria-label="Previous reference"
                            className="underline"
                            onClick={() =>
                              setReplayFocusIndex(
                                (current) =>
                                  (current - 1 + replayReferences.length) %
                                  replayReferences.length,
                              )
                            }
                            type="button"
                          >
                            Previous
                          </button>
                          <button
                            aria-label="Next reference"
                            className="underline"
                            onClick={() =>
                              setReplayFocusIndex(
                                (current) =>
                                  (current + 1) % replayReferences.length,
                              )
                            }
                            type="button"
                          >
                            Next
                          </button>
                        </span>
                        {fallbackHistorical.length > 0 ? (
                          <div className="mt-2 rounded-md border border-dashed p-2">
                            <strong>Historical references</strong>
                            {fallbackHistorical.map((item) => {
                              const key = boardReferenceKey(item.reference);
                              const current = replayFocusedKey === key;
                              return (
                                <p
                                  aria-current={current ? "true" : undefined}
                                  className={cn(
                                    "rounded px-1",
                                    current &&
                                      "font-bold outline outline-2 outline-offset-2",
                                  )}
                                  data-testid="historical-reference-placeholder"
                                  key={key}
                                >
                                  {item.reference.snapshot.label} · formerly{" "}
                                  {item.reference.placement.workstreamLabel} /{" "}
                                  {item.reference.placement.sectionLabel}
                                  {current ? " · Current" : ""}
                                </p>
                              );
                            })}
                          </div>
                        ) : null}
                      </div>
                    );
                  })()
                : null}
            </section>
          ) : null}
          {channelsQuery.isLoading ? (
            <p className="text-sm text-muted-foreground">
              Loading workstreams…
            </p>
          ) : channelsQuery.isError ? (
            <div className="flex flex-col items-center justify-center gap-2 py-16 text-muted-foreground">
              <p className="text-sm text-red-400">Failed to load channels</p>
              <Button
                onClick={() => void channelsQuery.refetch()}
                size="sm"
                variant="outline"
              >
                Retry
              </Button>
            </div>
          ) : sortedChannels.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No workstream channels found. Channels named "loganj-ws-…" will
              appear here.
            </p>
          ) : (
            <div className={WORKSTREAM_CARD_GRID_CLASS}>
              {sortedChannels.map((channel) => (
                <WorkstreamCard
                  activeWorking={activeTurnsByChannelId.get(channel.id)}
                  channel={channel}
                  discussionMode={discussionMode}
                  onToggleReference={toggleReference}
                  onReferencesChange={(references) =>
                    onReferencesChange(channel.id, references)
                  }
                  replayReferenceKeys={replayKeys}
                  replayFocusedReferenceKey={replayFocusedKey}
                  replayReferenceStates={replayReferenceStates}
                  historicalReplayReferences={resolvedReplayReferences
                    .filter(
                      (item) =>
                        item.state === "historical" &&
                        item.reference.placement.workstreamId === channel.id,
                    )
                    .map((item) => item.reference)}
                  selectedReferenceKeys={selectedKeys}
                  currentOwnerPubkey={identityQuery.data?.pubkey}
                  key={channel.id}
                  onSelect={(channelId) => void goChannel(channelId)}
                  onWaitsChange={(waits, createdAt) =>
                    onWaitsChange(channel.id, waits, createdAt)
                  }
                  profiles={activeProfilesQuery.data?.profiles}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
