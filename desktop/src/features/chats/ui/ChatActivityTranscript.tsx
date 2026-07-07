import * as React from "react";
import {
  AlertTriangle,
  Bot,
  Brain,
  CheckCircle2,
  ChevronDown,
  Circle,
  ClipboardList,
  Clock3,
  FileText,
  PlayCircle,
  ShieldQuestion,
} from "lucide-react";

import type {
  TranscriptDisplayBlock,
  TranscriptSameKindSummary,
  TranscriptTurnSegment,
} from "@/features/agents/ui/agentSessionTranscriptGrouping";
import type {
  PromptSection,
  TranscriptItem,
} from "@/features/agents/ui/agentSessionTypes";
import { getBuzzToolInfo } from "@/features/agents/ui/agentSessionToolCatalog";
import { buildCompactToolSummary } from "@/features/agents/ui/agentSessionToolSummary";
import { getToolDurationDisplay } from "@/features/agents/ui/agentSessionUtils";
import { RawEventRail } from "@/features/agents/ui/RawEventRail";
import { useObserverEvents } from "@/features/agents/ui/useObserverEvents";
import { ToolDetailBlocks } from "@/features/agents/ui/AgentSessionToolItem/ToolDetailBlocks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { ChatActivityRenderBlock } from "@/features/chats/lib/chatActivity";
import {
  ActivityMarkerRow,
  InlineActivityMarkerRow,
} from "@/features/chats/ui/ChatActivityMarkerRow";
import {
  activityItemLabel,
  activityItemTone,
  cleanChatMessageText,
  completedWorkLabel,
  isHumanFacingAssistantMessage,
  toolLabel,
} from "@/features/chats/ui/chatActivityText";
import {
  activityItemIcon as toolActivityItemIcon,
  getShellCommand,
  summaryIcon,
  toolCategoryIcon,
  toolIcon,
} from "@/features/chats/ui/chatActivityIcons";
import { isEntranceRecent } from "@/features/chats/ui/messageEntrance";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Bubble } from "@/shared/ui/bubble";
import { Markdown } from "@/shared/ui/markdown";
import {
  Message,
  MessageAvatar,
  MessageContent,
  MessageHeader,
} from "@/shared/ui/message";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function hasRecentEntrance(timestamp?: string | null) {
  return Boolean(timestamp) && isEntranceRecent(Date.parse(timestamp ?? ""));
}

// "Turn started" / "Session ready" plumbing rows. Hidden from the chat
// timeline (they add noise to every turn); their timestamps still feed the
// completed-turn duration.
function isSetupLifecycleItem(item: TranscriptItem) {
  if (item.type !== "lifecycle") {
    return false;
  }
  const source = item.acpSource ?? "";
  const title = item.title.toLowerCase();
  return (
    source === "turn_started" ||
    source === "session_resolved" ||
    title.includes("turn started") ||
    title.includes("session ready")
  );
}

// Memoized: rendered once per anchored message; placement recomputes on
// every transcript event, but rows whose props didn't change must not
// re-render their Markdown trees.
export const ChatActivityTranscript = React.memo(
  function ChatActivityTranscript({
    activeTurnIds,
    agent,
    blocks,
    identityPubkey,
    profiles,
    showAgentIdentity = true,
  }: {
    /** Turn ids currently live in this channel — drives per-turn rendering. */
    activeTurnIds?: ReadonlySet<string>;
    agent: ManagedAgent | null;
    blocks: ChatActivityRenderBlock[];
    identityPubkey?: string;
    profiles?: UserProfileLookup;
    /** Hidden in solo chats so agent replies read as part of the stream. */
    showAgentIdentity?: boolean;
  }) {
    if (blocks.length === 0) {
      return null;
    }

    return (
      <>
        {blocks.map((renderBlock) => (
          <ChatActivityBlockView
            agent={agent}
            block={renderBlock.block}
            identityPubkey={identityPubkey}
            isTurnActive={
              renderBlock.block.kind === "turn" &&
              (activeTurnIds?.has(renderBlock.block.turnId) ?? false)
            }
            key={renderBlock.id}
            profiles={profiles}
            showAgentIdentity={showAgentIdentity}
            suppressPromptMessage={renderBlock.suppressPromptMessage}
          />
        ))}
      </>
    );
  },
);

function ChatActivityBlockView({
  agent,
  block,
  identityPubkey,
  isTurnActive,
  profiles,
  showAgentIdentity,
  suppressPromptMessage,
}: {
  agent: ManagedAgent | null;
  block: TranscriptDisplayBlock;
  identityPubkey?: string;
  /** Whether THIS block's turn is live (per-turn, never channel-wide). */
  isTurnActive: boolean;
  profiles?: UserProfileLookup;
  showAgentIdentity: boolean;
  suppressPromptMessage: boolean;
}) {
  if (block.kind === "single") {
    return (
      <ChatActivityItemView
        agent={agent}
        identityPubkey={identityPubkey}
        item={block.item}
        profiles={profiles}
        showAgentIdentity={showAgentIdentity}
        suppressPromptMessage={suppressPromptMessage}
      />
    );
  }

  if (!isTurnActive && isCompletedTurn(block)) {
    return (
      <CompletedTurnView
        agent={agent}
        block={block}
        identityPubkey={identityPubkey}
        profiles={profiles}
        showAgentIdentity={showAgentIdentity}
        suppressPromptMessage={suppressPromptMessage}
      />
    );
  }

  return (
    <div data-chat-activity-turn={block.turnId}>
      {block.segments.map((segment) => (
        <ChatActivitySegmentView
          agent={agent}
          identityPubkey={identityPubkey}
          key={getSegmentKey(block.turnId, segment)}
          profiles={profiles}
          segment={segment}
          showAgentIdentity={showAgentIdentity}
          suppressPromptMessage={suppressPromptMessage}
        />
      ))}
      {isTurnActive && !hasLiveActivityItem(block) ? (
        <LiveTurnMarker
          agentPubkey={agent?.pubkey}
          icon={liveTurnMarkerIcon(block)}
          startedAt={getTurnStartedAt(block)}
          turnId={block.turnId}
        />
      ) : null}
    </div>
  );
}

function CompletedTurnView({
  agent,
  block,
  identityPubkey,
  profiles,
  showAgentIdentity,
  suppressPromptMessage,
}: {
  agent: ManagedAgent | null;
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>;
  identityPubkey?: string;
  profiles?: UserProfileLookup;
  showAgentIdentity: boolean;
  suppressPromptMessage: boolean;
}) {
  const promptSegments = block.segments.filter(
    (segment): segment is Extract<TranscriptTurnSegment, { kind: "prompt" }> =>
      segment.kind === "prompt",
  );
  const assistantMessages = collectFinalAssistantMessages(block);
  const activityItems = collectCompletedActivityItems(block);

  return (
    <div data-chat-activity-turn={block.turnId}>
      {promptSegments.map((segment) =>
        suppressPromptMessage ? null : (
          <ChatTranscriptMessageRow
            agent={agent}
            identityPubkey={identityPubkey}
            item={segment.user}
            key={`${block.turnId}:prompt:${segment.user.id}`}
            profiles={profiles}
          />
        ),
      )}
      {activityItems.length > 0 ? (
        <CompletedWorkMarker items={activityItems} />
      ) : null}
      {assistantMessages.map((item) => (
        <ChatTranscriptMessageRow
          agent={agent}
          identityPubkey={identityPubkey}
          item={item}
          key={item.id}
          profiles={profiles}
          showAgentIdentity={showAgentIdentity}
        />
      ))}
    </div>
  );
}

function getSegmentKey(turnId: string, segment: TranscriptTurnSegment) {
  if (segment.kind === "prompt") {
    return `${turnId}:prompt`;
  }
  if (segment.kind === "setup") {
    return `${turnId}:setup`;
  }
  if (segment.kind === "summary") {
    return segment.summary.id;
  }
  return segment.item.id;
}

function ChatActivitySegmentView({
  agent,
  identityPubkey,
  profiles,
  segment,
  showAgentIdentity,
  suppressPromptMessage,
}: {
  agent: ManagedAgent | null;
  identityPubkey?: string;
  profiles?: UserProfileLookup;
  segment: TranscriptTurnSegment;
  showAgentIdentity: boolean;
  suppressPromptMessage: boolean;
}) {
  if (segment.kind === "prompt") {
    return (
      <>
        {!suppressPromptMessage ? (
          <ChatTranscriptMessageRow
            agent={agent}
            identityPubkey={identityPubkey}
            item={segment.user}
            profiles={profiles}
          />
        ) : null}
        {segment.context ? (
          <ActivityMarkerRow
            details={<PromptSections sections={segment.context.sections} />}
            entrance={hasRecentEntrance(segment.context.timestamp)}
            icon={<FileText className="h-3.5 w-3.5" />}
            label="Captured prompt context"
            timestamp={segment.context.timestamp}
            tone="muted"
          />
        ) : null}
      </>
    );
  }

  if (segment.kind === "setup") {
    // "Turn started" / "Session ready" plumbing rows add noise to every
    // turn without telling the user anything actionable — the Working
    // marker (and its raw-event dropdown) already covers turn liveness.
    return null;
  }

  if (segment.kind === "summary") {
    return <SummaryMarker summary={segment.summary} />;
  }

  return (
    <ChatActivityItemView
      agent={agent}
      identityPubkey={identityPubkey}
      item={segment.item}
      profiles={profiles}
      showAgentIdentity={showAgentIdentity}
      suppressPromptMessage={suppressPromptMessage}
    />
  );
}

function ChatActivityItemView({
  agent,
  identityPubkey,
  item,
  profiles,
  showAgentIdentity,
  suppressPromptMessage,
}: {
  agent: ManagedAgent | null;
  identityPubkey?: string;
  item: TranscriptItem;
  profiles?: UserProfileLookup;
  showAgentIdentity: boolean;
  suppressPromptMessage: boolean;
}) {
  if (item.type === "message") {
    if (item.role === "user" && suppressPromptMessage) {
      return null;
    }
    if (item.role === "assistant" && !isHumanFacingAssistantMessage(item)) {
      return null;
    }
    if (cleanChatMessageText(item).length === 0) return null;
    return (
      <ChatTranscriptMessageRow
        agent={agent}
        identityPubkey={identityPubkey}
        item={item}
        profiles={profiles}
        showAgentIdentity={showAgentIdentity}
      />
    );
  }

  if (item.type === "tool") {
    return <ToolMarker item={item} />;
  }

  if (item.type === "thought") {
    return (
      <ActivityMarkerRow
        details={<Markdown content={item.text.trim() || " "} />}
        entrance={hasRecentEntrance(item.timestamp)}
        icon={<Brain className="h-3.5 w-3.5" />}
        label={item.title || "Thinking"}
        timestamp={item.timestamp}
        tone="muted"
      />
    );
  }

  if (item.type === "plan") {
    const label = item.isUpdate
      ? item.text
        ? `Updated plan · ${item.text}`
        : "Updated plan"
      : "Updated plan";
    return (
      <ActivityMarkerRow
        details={
          item.isUpdate ? null : (
            <Markdown content={item.text.trim() || "No plan details."} />
          )
        }
        entrance={hasRecentEntrance(item.timestamp)}
        icon={<ClipboardList className="h-3.5 w-3.5" />}
        label={label}
        timestamp={item.timestamp}
        tone="default"
      />
    );
  }

  if (item.type === "metadata") {
    return (
      <ActivityMarkerRow
        details={<PromptSections sections={item.sections} />}
        entrance={hasRecentEntrance(item.timestamp)}
        icon={<FileText className="h-3.5 w-3.5" />}
        label={`Captured ${item.title.toLowerCase()}`}
        timestamp={item.timestamp}
        tone="muted"
      />
    );
  }

  if (isSetupLifecycleItem(item)) {
    return null;
  }

  return (
    <ActivityMarkerRow
      details={item.text ? <Markdown content={item.text} /> : null}
      entrance={hasRecentEntrance(item.timestamp)}
      icon={lifecycleIcon(item)}
      label={item.title}
      timestamp={item.timestamp}
      tone={item.renderClass === "error" ? "danger" : "muted"}
    />
  );
}

function ChatTranscriptMessageRow({
  agent,
  identityPubkey,
  item,
  profiles,
  showAgentIdentity = true,
}: {
  agent: ManagedAgent | null;
  identityPubkey?: string;
  item: Extract<TranscriptItem, { type: "message" }>;
  profiles?: UserProfileLookup;
  showAgentIdentity?: boolean;
}) {
  const isUser = item.role === "user";
  const pubkey = isUser
    ? (item.authorPubkey ?? identityPubkey ?? "")
    : (agent?.pubkey ?? item.authorPubkey ?? "");
  const label = pubkey
    ? resolveUserLabel({
        pubkey,
        currentPubkey: identityPubkey,
        fallbackName: isUser ? item.title || "You" : agent?.name || "Fizz",
        profiles,
        preferResolvedSelfLabel: !isUser,
      })
    : isUser
      ? item.title || "You"
      : agent?.name || "Fizz";
  const profile = pubkey ? profiles?.[normalizePubkey(pubkey)] : null;
  const avatarUrl = isUser
    ? (profile?.avatarUrl ?? null)
    : (profile?.avatarUrl ?? agent?.avatarUrl ?? null);
  const text = item.text.trim();
  const displayText = cleanChatMessageText(item);
  const entrance = hasRecentEntrance(item.timestamp);

  const hideIdentity = !isUser && !showAgentIdentity;

  return (
    <Message
      className={cn(entrance && "buzz-message-entrance")}
      side={isUser ? "right" : "left"}
    >
      {!isUser && !hideIdentity ? (
        <MessageAvatar>
          <UserAvatar avatarUrl={avatarUrl} displayName={label} size="sm" />
        </MessageAvatar>
      ) : null}
      <MessageContent className={isUser ? "items-end" : "w-full max-w-full"}>
        {!hideIdentity && !isUser ? (
          <MessageHeader>
            <span className="truncate font-medium">{label}</span>
          </MessageHeader>
        ) : null}
        {isUser ? (
          <Bubble side="right">
            <Markdown
              className="min-w-0 font-medium [&_*]:text-primary-foreground [&_a]:text-primary-foreground [&_code]:bg-primary-foreground/15 [&_code]:text-primary-foreground"
              content={displayText || text || " "}
            />
          </Bubble>
        ) : (
          <Markdown
            agentAuthored
            className="w-full max-w-none text-sm font-medium leading-6"
            content={displayText || text || " "}
          />
        )}
      </MessageContent>
    </Message>
  );
}

function SummaryMarker({ summary }: { summary: TranscriptSameKindSummary }) {
  return (
    <ActivityMarkerRow
      details={
        <div className="space-y-1.5">
          {summary.items.map((item) => (
            <ActivityPreviewLine item={item} key={item.id} />
          ))}
        </div>
      }
      entrance={hasRecentEntrance(summary.timestamp)}
      icon={summaryIcon(summary)}
      label={summary.label}
      timestamp={summary.timestamp}
      tone="default"
    />
  );
}

function CompletedWorkMarker({ items }: { items: TranscriptItem[] }) {
  const [isOpen, setIsOpen] = React.useState(false);
  // Duration spans ALL items (setup lifecycle marks the turn's true start),
  // but the expanded list hides the "Turn started"/"Session ready" plumbing.
  const label = completedWorkLabel(items);
  const detailItems = items.filter((item) => !isSetupLifecycleItem(item));
  const entrance = hasRecentEntrance(items[items.length - 1]?.timestamp);

  return (
    <Message
      className={cn("px-0 py-1", entrance && "buzz-message-entrance")}
      side="left"
    >
      <MessageContent className="w-full max-w-full">
        <button
          aria-expanded={isOpen}
          className="block w-full min-w-0 text-left"
          onClick={() => setIsOpen((current) => !current)}
          type="button"
        >
          <span className="flex min-w-0 items-center gap-1.5 text-muted-foreground/70">
            <span className="truncate text-sm font-medium">{label}</span>
            <ChevronDown
              className={cn(
                "h-4 w-4 shrink-0 text-muted-foreground/70 transition-transform",
                isOpen && "rotate-180",
              )}
            />
          </span>
        </button>
        <div className="mt-3 border-t border-border/70" />
        {isOpen ? (
          <div className="pt-4">
            <div className="space-y-3">
              {detailItems.map((item) => (
                <CompletedWorkDetailRow item={item} key={item.id} />
              ))}
            </div>
          </div>
        ) : null}
      </MessageContent>
    </Message>
  );
}

// How many of the turn's most recent raw observer events the "Working"
// dropdown shows. Enough to read the current backend activity without
// rendering the whole turn's wire history.
const LIVE_TURN_RAW_EVENT_LIMIT = 12;

function LiveTurnMarker({
  agentPubkey,
  icon,
  startedAt,
  turnId,
}: {
  agentPubkey?: string | null;
  icon: React.ReactNode;
  startedAt: string | null;
  turnId: string;
}) {
  const elapsedSeconds = useElapsedSeconds(startedAt);
  const { events } = useObserverEvents(Boolean(agentPubkey), agentPubkey);
  const turnEvents = React.useMemo(
    () =>
      events
        .filter((event) => event.turnId === turnId)
        .slice(-LIVE_TURN_RAW_EVENT_LIMIT),
    [events, turnId],
  );

  return (
    <ActivityMarkerRow
      details={
        turnEvents.length > 0 ? (
          <div className="max-h-80 overflow-y-auto pr-1">
            <RawEventRail events={turnEvents} />
          </div>
        ) : (
          <span>Waiting for the first backend event…</span>
        )
      }
      // No entrance animation: this row mounts and unmounts with every gap
      // between tool executions, and replaying the fade on each reappearance
      // reads as blinking instead of a steady status.
      icon={icon}
      label="Working"
      loading
      meta={formatElapsedCounter(elapsedSeconds)}
      timestamp={startedAt ?? undefined}
      tone="muted"
    />
  );
}

function CompletedWorkDetailRow({ item }: { item: TranscriptItem }) {
  const details = activityItemDetails(item);
  return (
    <InlineActivityMarkerRow
      details={details}
      icon={activityItemIcon(item)}
      label={activityItemLabel(item)}
      loading={isActivityItemLive(item)}
      timestamp={item.timestamp}
      tone={activityItemTone(item)}
    />
  );
}

function ToolMarker({
  item,
}: {
  item: Extract<TranscriptItem, { type: "tool" }>;
}) {
  const compactSummary = buildCompactToolSummary(item);
  const hasArgs = Object.keys(item.args).length > 0;
  const hasResult = item.result.trim().length > 0;
  const canonicalToolName = item.buzzToolName ?? item.toolName;
  const buzzTool = getBuzzToolInfo(canonicalToolName);
  const shellCommand = getShellCommand(item, compactSummary);
  const showDetails =
    hasArgs ||
    hasResult ||
    compactSummary.fileEditDiff !== null ||
    compactSummary.thumbnailSrc !== null ||
    shellCommand !== null;

  return (
    <ActivityMarkerRow
      details={
        showDetails ? (
          <ToolDetailBlocks
            args={item.args}
            description={buzzTool?.label}
            fileEditDiff={compactSummary.fileEditDiff}
            fileReadContent={compactSummary.fileReadContent}
            hasArgs={hasArgs}
            hasResult={hasResult}
            imagePreview={
              compactSummary.thumbnailSrc
                ? {
                    src: compactSummary.thumbnailSrc,
                    title: compactSummary.preview,
                  }
                : null
            }
            isError={item.isError || item.status === "failed"}
            result={item.result}
            shellCommand={shellCommand}
          />
        ) : null
      }
      entrance={hasRecentEntrance(item.timestamp)}
      icon={toolIcon(item, compactSummary)}
      label={toolLabel(item)}
      loading={isToolRunning(item)}
      meta={getToolDurationDisplay(item)}
      timestamp={item.timestamp}
      tone={
        item.isError || item.status === "failed"
          ? "danger"
          : item.status === "executing" || item.status === "pending"
            ? "warning"
            : "default"
      }
    />
  );
}

function ActivityPreviewLine({ item }: { item: TranscriptItem }) {
  if (item.type === "tool") {
    const summary = buildCompactToolSummary(item);
    return (
      <div className="flex min-w-0 items-center gap-2 text-sm">
        <span className="min-w-0 truncate text-muted-foreground">
          {toolLabel(item)}
        </span>
        {summary.preview && summary.kind !== "shell" ? (
          <span className="min-w-0 truncate text-muted-foreground/80">
            {summary.preview}
          </span>
        ) : null}
      </div>
    );
  }

  return (
    <div className="truncate text-sm text-muted-foreground">
      {activityItemLabel(item)}
    </div>
  );
}

function activityItemDetails(item: TranscriptItem) {
  if (item.type === "tool") {
    const compactSummary = buildCompactToolSummary(item);
    const hasArgs = Object.keys(item.args).length > 0;
    const hasResult = item.result.trim().length > 0;
    const canonicalToolName = item.buzzToolName ?? item.toolName;
    const buzzTool = getBuzzToolInfo(canonicalToolName);
    const shellCommand = getShellCommand(item, compactSummary);
    return (
      <ToolDetailBlocks
        args={item.args}
        description={buzzTool?.label}
        fileEditDiff={compactSummary.fileEditDiff}
        fileReadContent={compactSummary.fileReadContent}
        hasArgs={hasArgs}
        hasResult={hasResult}
        imagePreview={
          compactSummary.thumbnailSrc
            ? {
                src: compactSummary.thumbnailSrc,
                title: compactSummary.preview,
              }
            : null
        }
        isError={item.isError || item.status === "failed"}
        result={item.result}
        shellCommand={shellCommand}
      />
    );
  }
  if (item.type === "metadata") {
    return <PromptSections sections={item.sections} />;
  }
  if (item.type === "plan" || item.type === "thought") {
    return <Markdown content={item.text.trim() || " "} />;
  }
  if (item.type === "lifecycle") {
    return item.text ? <Markdown content={item.text} /> : null;
  }
  return null;
}

function PromptSections({ sections }: { sections: PromptSection[] }) {
  if (sections.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">No context captured.</p>
    );
  }

  return (
    <div className="space-y-2">
      {sections.map((section) => (
        <details
          className="group/context overflow-hidden rounded-md bg-muted/35"
          key={`${section.title}:${section.body.slice(0, 48)}`}
        >
          <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-sm font-medium">
            <span className="min-w-0 flex-1 truncate">{section.title}</span>
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground opacity-0 transition-all group-hover/context:opacity-100 group-open/context:rotate-180 group-open/context:opacity-100" />
          </summary>
          <div className="px-3 pb-3 text-sm text-muted-foreground">
            <Markdown content={section.body.trim() || "No metadata."} />
          </div>
        </details>
      ))}
    </div>
  );
}

function isCompletedTurn(
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>,
) {
  if (!collectAssistantMessages(block).some((item) => item.text.trim())) {
    return false;
  }

  return !collectTurnItems(block).some(
    (item) =>
      item.type === "tool" &&
      (item.status === "executing" || item.status === "pending"),
  );
}

function collectAssistantMessages(
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>,
) {
  return collectTurnItems(block).filter(
    (item): item is Extract<TranscriptItem, { type: "message" }> =>
      item.type === "message" && item.role === "assistant",
  );
}

function collectFinalAssistantMessages(
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>,
) {
  const messages = collectAssistantMessages(block).filter(
    isHumanFacingAssistantMessage,
  );
  const finalMessage = messages[messages.length - 1];
  return finalMessage ? [finalMessage] : [];
}

function collectCompletedActivityItems(
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>,
) {
  const items: TranscriptItem[] = [];
  for (const segment of block.segments) {
    if (segment.kind === "prompt") {
      items.push(...segment.setup);
      if (segment.context) {
        items.push(segment.context);
      }
    } else if (segment.kind === "setup") {
      items.push(...segment.items);
    } else if (segment.kind === "summary") {
      items.push(...segment.summary.items);
    } else if (
      segment.item.type !== "message" ||
      segment.item.role !== "assistant"
    ) {
      items.push(segment.item);
    }
  }
  return items.filter((item) => activityItemLabel(item).length > 0);
}

function collectTurnItems(
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>,
) {
  const items: TranscriptItem[] = [];
  for (const segment of block.segments) {
    if (segment.kind === "prompt") {
      items.push(segment.user, ...segment.setup);
      if (segment.context) {
        items.push(segment.context);
      }
    } else if (segment.kind === "setup") {
      items.push(...segment.items);
    } else if (segment.kind === "summary") {
      items.push(...segment.summary.items);
    } else {
      items.push(segment.item);
    }
  }
  return items;
}

function isActivityItemLive(item: TranscriptItem) {
  return item.type === "tool" && isToolRunning(item);
}

function hasLiveActivityItem(
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>,
) {
  return collectTurnItems(block).some(isActivityItemLive);
}

function liveTurnMarkerIcon(
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>,
) {
  const latestTool = [...collectTurnItems(block)]
    .reverse()
    .find(
      (item): item is Extract<TranscriptItem, { type: "tool" }> =>
        item.type === "tool",
    );
  if (!latestTool) {
    return <Clock3 className="h-3.5 w-3.5" />;
  }
  return toolCategoryIcon(latestTool, buildCompactToolSummary(latestTool));
}

function getTurnStartedAt(
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>,
) {
  const items = collectTurnItems(block);
  const turnStarted = items.find(
    (item): item is Extract<TranscriptItem, { type: "lifecycle" }> =>
      item.type === "lifecycle" &&
      ((item.acpSource ?? "") === "turn_started" ||
        item.title.toLowerCase().includes("turn started")),
  );
  if (turnStarted) {
    return turnStarted.timestamp;
  }

  const timestamps = items
    .map((item) => Date.parse(item.timestamp))
    .filter((value) => Number.isFinite(value));
  if (timestamps.length === 0) {
    return null;
  }
  return new Date(Math.min(...timestamps)).toISOString();
}

function useElapsedSeconds(startedAt: string | null) {
  const [now, setNow] = React.useState(() => Date.now());

  React.useEffect(() => {
    const interval = window.setInterval(() => {
      setNow(Date.now());
    }, 1000);
    return () => window.clearInterval(interval);
  }, []);

  if (!startedAt) {
    return null;
  }

  const start = Date.parse(startedAt);
  if (!Number.isFinite(start)) {
    return null;
  }
  return Math.max(0, Math.floor((now - start) / 1000));
}

function formatElapsedCounter(seconds: number | null) {
  if (seconds === null) {
    return null;
  }
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return remainder > 0 ? `${minutes}m ${remainder}s` : `${minutes}m`;
}

function isToolRunning(item: Extract<TranscriptItem, { type: "tool" }>) {
  return item.status === "executing" || item.status === "pending";
}

function activityItemIcon(item: TranscriptItem) {
  const toolIconNode = toolActivityItemIcon(item);
  if (toolIconNode) {
    return toolIconNode;
  }
  if (item.type === "metadata") {
    return <FileText className="h-3.5 w-3.5" />;
  }
  if (item.type === "plan") {
    return <ClipboardList className="h-3.5 w-3.5" />;
  }
  if (item.type === "thought") {
    return <Brain className="h-3.5 w-3.5" />;
  }
  if (item.type === "lifecycle") {
    return lifecycleIcon(item);
  }
  return <CheckCircle2 className="h-3.5 w-3.5" />;
}

function lifecycleIcon(item: Extract<TranscriptItem, { type: "lifecycle" }>) {
  const source = item.acpSource ?? "";
  const title = item.title.toLowerCase();
  if (source === "turn_started" || title.includes("turn started")) {
    return <PlayCircle className="h-3.5 w-3.5" />;
  }
  if (source === "session_resolved" || title.includes("session ready")) {
    return <CheckCircle2 className="h-3.5 w-3.5" />;
  }
  const renderClass = item.renderClass;
  if (renderClass === "permission") {
    return <ShieldQuestion className="h-3.5 w-3.5" />;
  }
  if (renderClass === "error") {
    return <AlertTriangle className="h-3.5 w-3.5" />;
  }
  if (renderClass === "status") {
    return <Circle className="h-3.5 w-3.5" />;
  }
  return <Bot className="h-3.5 w-3.5" />;
}
