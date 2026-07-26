import * as React from "react";
import { Octagon, X } from "lucide-react";

import * as BuzzTheme from "@/app/BuzzThemeSurfaces";

import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  buzzwordAt,
  deriveHarnessStatus,
  formatElapsed,
  formatTokens,
  type HarnessStatusItem,
} from "./harnessStatus";

/**
 * A person currently viewing this harness session.
 *
 * Presence is derived from the community presence stream (kind 20001), so
 * "in harness" means "a member of this channel, with their community presence"
 * rather than "has the harness view open" — the relay does not model per-view
 * presence.
 */
export type HarnessParticipant = {
  pubkey: string;
  displayName: string;
  avatarUrl: string | null;
  status: PresenceStatus;
  isSelf: boolean;
};

/**
 * A permission request awaiting a human decision.
 *
 * Inert until the harness stops auto-approving `session/request_permission`
 * (`crates/buzz-acp/src/acp.rs` currently answers `allow_once` immediately).
 * Rendering it here keeps the layout honest about where the decision lands
 * once grant/deny (kinds 46030 / 46031) are wired.
 */
export type HarnessPendingApproval = {
  id: string;
  title: string;
  detail?: string | null;
};

/** A human message from the thread, shown as the query history rail. */
export type HarnessThreadMessage = {
  id: string;
  author: string;
  avatarUrl: string | null;
  body: string;
  time: string;
  /** Nostr seconds — used to order the rail newest-first. */
  createdAt: number;
  /** Author pubkey, so a pending bubble matches its in-transcript colour. */
  authorPubkey: string | null;
  isSelf: boolean;
};

type HarnessModeScreenProps = {
  agent: React.ComponentProps<typeof ManagedAgentSessionPanel>["agent"];
  channelId: string | null;
  channelName: string | null;
  participants: readonly HarnessParticipant[];
  isWorking: boolean;
  /** Gate destructive turn cancellation to admins / the turn's initiator. */
  canCancelTurn: boolean;
  onCancelTurn?: () => void;
  onExit: () => void;
  composerDisabled?: boolean;
  isSending?: boolean;
  /** Human messages in the originating thread, oldest first. */
  threadMessages?: readonly HarnessThreadMessage[];
  /** Participants currently composing, from the channel's typing stream. */
  typingParticipants?: readonly HarnessParticipant[];
  /**
   * Event ids of this thread's messages. Scopes the transcript to the turns
   * they started, keeping threads in the same channel independent.
   */
  threadMessageIds?: ReadonlySet<string>;
  /** Extra transcript rows to interleave — the agent's published replies. */
  extraTranscriptItems?: React.ComponentProps<
    typeof ManagedAgentSessionPanel
  >["extraTranscriptItems"];
  /**
   * Sends a message into this harness's channel. The agent's pubkey is always
   * appended to `mentionPubkeys` before this fires — see `handleSend`.
   */
  onSend?: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    channelId?: string | null,
  ) => Promise<void>;
  pendingApproval?: HarnessPendingApproval | null;
  onApprove?: (id: string) => void;
  onDeny?: (id: string) => void;
  profiles?: UserProfileLookup;
};

// Matches the members sidebar's section heading treatment so the rail reads as
// part of the same design system rather than a bespoke panel.
const BUZZWORD_INTERVAL_SECONDS = 5;

const RAIL_DEFAULT_WIDTH = 320;
const RAIL_MIN_WIDTH = 220;
const RAIL_MAX_WIDTH = 520;

const RAIL_SECTION_LABEL =
  "text-sm font-semibold tracking-tight text-muted-foreground";

export function HarnessModeScreen({
  agent,
  channelId,
  channelName,
  participants,
  isWorking,
  canCancelTurn,
  onCancelTurn,
  onExit,
  composerDisabled = false,
  isSending = false,
  onSend,
  threadMessages,
  typingParticipants,
  threadMessageIds,
  extraTranscriptItems,
  pendingApproval = null,
  onApprove,
  onDeny,
  profiles,
}: HarnessModeScreenProps) {
  // Escape exits the harness rather than closing the whole window.
  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onExit();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onExit]);

  // ── Live status strip ──────────────────────────────────────────────────────
  const [statusItems, setStatusItems] = React.useState<HarnessStatusItem[]>([]);
  const onTranscriptChange = React.useCallback((items: HarnessStatusItem[]) => {
    setStatusItems(items);
  }, []);
  const status = React.useMemo(
    () => deriveHarnessStatus(statusItems),
    [statusItems],
  );

  // One interval drives both the elapsed clock and the buzzword cycle, so the
  // word changes on a visible beat rather than every render.
  const [tick, setTick] = React.useState(0);
  const turnStartRef = React.useRef<number | null>(null);
  const [elapsedMs, setElapsedMs] = React.useState(0);

  React.useEffect(() => {
    if (!isWorking) {
      turnStartRef.current = null;
      return;
    }
    if (turnStartRef.current === null) {
      turnStartRef.current = performance.now();
    }
    let seconds = 0;
    const id = window.setInterval(() => {
      seconds += 1;
      // Elapsed updates every second; the buzzword changes every 5 so it reads
      // as a status rather than a flicker.
      if (seconds % BUZZWORD_INTERVAL_SECONDS === 0) {
        setTick((value) => value + 1);
      }
      if (turnStartRef.current !== null) {
        setElapsedMs(performance.now() - turnStartRef.current);
      }
    }, 1000);
    return () => window.clearInterval(id);
  }, [isWorking]);

  // Rendered beside the liveness bees rather than as its own strip, so the
  // "what it's doing" reads as one line with the animation.
  const liveStatus = (
    <span
      className="flex min-w-0 items-baseline gap-1.5 text-sm"
      data-testid="harness-status-line"
    >
      <span className="shrink-0 text-primary">{buzzwordAt(tick)}…</span>
      <span className="shrink-0 text-xs text-muted-foreground">
        {formatElapsed(elapsedMs)}
        {status.tokensUsed !== null
          ? ` · ↓ ${formatTokens(status.tokensUsed)} tokens`
          : ""}
        {status.toolsTotal > 0
          ? ` · ${status.toolsDone}/${status.toolsTotal} tools`
          : ""}
      </span>
      {status.summary ? (
        <span className="truncate text-xs italic text-muted-foreground/70">
          {status.summary}
        </span>
      ) : null}
    </span>
  );

  // Rail width is drag-resizable within bounds. Clamped rather than free so the
  // transcript always keeps a readable column.
  const [railWidth, setRailWidth] = React.useState(RAIL_DEFAULT_WIDTH);
  const dragStateRef = React.useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const mainRef = React.useRef<HTMLElement>(null);

  const onDragStart = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      event.preventDefault();
      dragStateRef.current = { startX: event.clientX, startWidth: railWidth };
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [railWidth],
  );

  const onDragMove = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const drag = dragStateRef.current;
      if (!drag) {
        return;
      }
      const next = drag.startWidth + (event.clientX - drag.startX);
      setRailWidth(Math.min(RAIL_MAX_WIDTH, Math.max(RAIL_MIN_WIDTH, next)));
    },
    [],
  );

  const onDragEnd = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      dragStateRef.current = null;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    },
    [],
  );

  // Newest first: the rail is a "what just happened" log, so the latest query
  // should be reachable without scrolling to the bottom of a long thread.
  const railMessages = React.useMemo(
    () =>
      threadMessages
        ? [...threadMessages].sort((a, b) => b.createdAt - a.createdAt)
        : undefined,
    [threadMessages],
  );

  // Scroll the transcript to a rail row's anchor. Queried live rather than held
  // in a ref map because transcript rows mount and unmount as the window scrolls.
  const presentCount = participants.filter(
    (participant) => participant.status !== "offline",
  ).length;

  // In harness mode you are, by definition, addressing this agent — so the
  // agent is always p-tagged even when the text carries no literal @mention.
  // Without it the relay never routes the message to the harness's
  // `#p`-filtered subscription and the turn silently never starts.
  const handleSend = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      sendChannelId?: string | null,
    ) => {
      if (!onSend) {
        return;
      }
      const withAgent = mentionPubkeys.some(
        (pubkey) => pubkey.toLowerCase() === agent.pubkey.toLowerCase(),
      )
        ? mentionPubkeys
        : [...mentionPubkeys, agent.pubkey];

      await onSend(content, withAgent, mediaTags, sendChannelId ?? channelId);
    },
    [agent.pubkey, channelId, onSend],
  );

  return (
    // Sits below the global top chrome rather than over it: that strip hosts the
    // macOS traffic lights and drag region, so covering it (inset-0) puts the
    // header under the window controls. Offsetting by the same CSS var the rest
    // of the app uses keeps the harness aligned with every other surface.
    <div
      className={cn(
        // Above the auxiliary panes, which sit at z-50 (RightAuxiliaryPane):
        // at z-40 the open thread pane rendered over the harness and its edge
        // showed as a stray vertical rule through the transcript. Still below
        // the app's top-most layer (z-[100]) so dialogs and toasts win.
        // bg-background is the opaque base, NOT decoration: this is an overlay
        // above the live channel view, so without it the sidebar and channel
        // header show straight through the transparent rail. GradientLayer sits
        // at -z-10, which paints over this background but under the content —
        // the same base → gradient → content order the app shell uses.
        "fixed inset-x-0 bottom-0 z-[60] flex flex-col bg-background",
        "top-(--buzz-top-chrome-height,40px)",
      )}
    >
      {/* Same two-layer surface the app shell uses: the brand gradient behind,
          content floating on a rounded card. Without these the harness reads as
          a flat modal bolted onto Buzz rather than one of its screens. */}
      <BuzzTheme.GradientLayer />
      <header className="flex items-center gap-3 px-5 py-3">
        <UserAvatar
          avatarUrl={agent.avatarUrl ?? null}
          displayName={agent.name}
          size="sm"
        />
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm font-medium tracking-tight">
            {agent.name}
            {channelName ? (
              <span className="font-normal text-muted-foreground">
                {" "}
                · #{channelName}
              </span>
            ) : null}
          </span>
          <span className="text-xs text-muted-foreground">
            Harness · Esc to return to the thread
          </span>
        </div>

        <div className="ml-auto flex items-center gap-2">
          <Badge variant={isWorking ? "default" : "secondary"}>
            {isWorking ? "Processing" : "Idle"}
          </Badge>
          {canCancelTurn && isWorking ? (
            <Button
              onClick={onCancelTurn}
              size="sm"
              type="button"
              variant="outline"
            >
              <Octagon aria-hidden className="size-3.5" />
              Stop turn
            </Button>
          ) : null}
          <Button
            aria-label="Exit harness mode"
            onClick={onExit}
            size="icon"
            type="button"
            variant="ghost"
          >
            <X aria-hidden className="size-4" />
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <aside
          className="flex shrink-0 flex-col gap-6 overflow-y-auto px-4 py-4"
          style={{ width: railWidth }}
        >
          <section className="flex flex-col gap-2.5">
            <div className="flex items-baseline justify-between gap-2">
              <h2 className={RAIL_SECTION_LABEL}>In harness</h2>
              <span className="text-xs text-muted-foreground">
                {presentCount}/{participants.length}
              </span>
            </div>
            <ul className="flex flex-col gap-1">
              {participants.length === 0 ? (
                <li className="text-sm text-muted-foreground">No members</li>
              ) : (
                participants.map((participant) => (
                  <li
                    className="flex items-center gap-2 rounded-md px-1 py-1"
                    key={participant.pubkey}
                  >
                    <span className="relative shrink-0">
                      <UserAvatar
                        avatarUrl={participant.avatarUrl}
                        displayName={participant.displayName}
                        size="xs"
                      />
                      <PresenceDot
                        className="absolute -right-0.5 -bottom-0.5 ring-2 ring-background"
                        status={participant.status}
                      />
                    </span>
                    <span
                      className={cn(
                        "truncate text-sm font-medium tracking-tight",
                        participant.status === "offline" &&
                          "text-muted-foreground",
                      )}
                    >
                      {participant.displayName}
                    </span>
                    {participant.isSelf ? (
                      <span className="shrink-0 text-xs text-muted-foreground">
                        you
                      </span>
                    ) : null}
                  </li>
                ))
              )}
            </ul>
          </section>

          {typingParticipants && typingParticipants.length > 0 ? (
            <section
              className="flex animate-pulse flex-col gap-2 rounded-lg border border-primary/40 bg-primary/10 p-3"
              data-testid="harness-typing-banner"
            >
              <div className="flex items-center gap-2">
                <span className="flex gap-0.5" aria-hidden>
                  {[0, 1, 2].map((dot) => (
                    <span
                      className="size-1.5 rounded-full bg-primary"
                      key={dot}
                    />
                  ))}
                </span>
                <span className="text-sm font-medium tracking-tight text-primary">
                  {typingParticipants.length === 1
                    ? `${typingParticipants[0].displayName} is typing…`
                    : `${typingParticipants.length} people are typing…`}
                </span>
              </div>
              <p className="text-xs text-muted-foreground">
                Incoming — it lands in the queue when sent.
              </p>
            </section>
          ) : null}

          {pendingApproval ? (
            <section className="flex flex-col gap-2.5 rounded-lg border border-border bg-muted/30 p-3">
              <h2 className={RAIL_SECTION_LABEL}>Approval needed</h2>
              <p className="text-sm font-medium break-words">
                {pendingApproval.title}
              </p>
              {pendingApproval.detail ? (
                <p className="break-words text-xs text-muted-foreground">
                  {pendingApproval.detail}
                </p>
              ) : null}
              <div className="flex gap-2">
                <Button
                  onClick={() => onApprove?.(pendingApproval.id)}
                  size="sm"
                  type="button"
                >
                  Approve
                </Button>
                <Button
                  onClick={() => onDeny?.(pendingApproval.id)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Deny
                </Button>
              </div>
            </section>
          ) : null}

          {railMessages ? (
            <section className="flex min-h-0 flex-col gap-2.5">
              <div className="flex items-baseline justify-between gap-2">
                <h2 className={RAIL_SECTION_LABEL}>Thread history</h2>
                <span className="text-xs text-muted-foreground">
                  {railMessages.length}
                </span>
              </div>
              {railMessages.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No messages in this thread yet.
                </p>
              ) : (
                <ol className="flex flex-col gap-1">
                  {railMessages.map((message) => (
                    <li className="flex gap-2 px-1 py-1.5" key={message.id}>
                      <UserAvatar
                        avatarUrl={message.avatarUrl}
                        className="mt-0.5 shrink-0"
                        displayName={message.author}
                        size="xs"
                      />
                      <div className="flex min-w-0 flex-col gap-0.5">
                        <div className="flex items-baseline gap-1.5">
                          <span className="truncate text-sm font-medium tracking-tight">
                            {message.author}
                          </span>
                          <span className="shrink-0 text-xs text-muted-foreground">
                            {message.time}
                          </span>
                        </div>
                        <p className="line-clamp-2 break-words text-sm text-muted-foreground">
                          {message.body}
                        </p>
                      </div>
                    </li>
                  ))}
                </ol>
              )}
            </section>
          ) : null}
        </aside>

        {/* Pointer-capture drag rather than a library: the handle is a thin
            seam that reveals a rule on hover, matching the app's pane resizers. */}
        <div
          aria-hidden="true"
          className="group relative w-1 shrink-0 cursor-col-resize"
          data-testid="harness-rail-resize"
          onPointerCancel={onDragEnd}
          onPointerDown={onDragStart}
          onPointerMove={onDragMove}
          onPointerUp={onDragEnd}
        >
          <div className="absolute inset-y-0 left-0 w-px bg-transparent transition-colors group-hover:bg-border" />
        </div>

        <BuzzTheme.ContentSurface>
          {/* min-h-0 is load-bearing: without it this column flex child cannot
              shrink below its content height, so the transcript's own scroll
              container never gets a bounded height and ContentSurface's
              overflow-hidden clips the tail instead of letting it scroll. */}
          <main className="flex min-h-0 min-w-0 flex-1 flex-col" ref={mainRef}>
            {/* showRaw={false}: the raw JSON-RPC rail is a debugging surface, not
              part of the shared session view. It belongs behind the existing
              per-panel toggle, not pinned open in a full-screen room. */}
            <div className="min-h-0 flex-1">
              <ManagedAgentSessionPanel
                agent={agent}
                autoTail
                channelId={channelId}
                className="h-full"
                emptyDescription="Send a message below to start a turn."
                profiles={profiles}
                showHeader={false}
                hideTrailingNarration
                liveStatusSlot={liveStatus}
                onTranscriptChange={onTranscriptChange}
                showRaw={false}
                extraTranscriptItems={extraTranscriptItems}
                threadMessageIds={threadMessageIds}
              />
            </div>
            {onSend ? (
              <div className="flex shrink-0 flex-col gap-2 border-t border-border/35 px-4 py-3">
                <MessageComposer
                  channelId={channelId}
                  channelName={channelName ?? "channel"}
                  disabled={composerDisabled}
                  draftKey={`harness:${agent.pubkey}:${channelId ?? "none"}`}
                  isSending={isSending}
                  onSend={handleSend}
                  placeholder={`Message ${agent.name}`}
                  profiles={profiles}
                />
              </div>
            ) : null}
          </main>
        </BuzzTheme.ContentSurface>
      </div>
    </div>
  );
}
