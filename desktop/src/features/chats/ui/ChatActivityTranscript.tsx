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

export function ChatActivityTranscript({
  agent,
  blocks,
  identityPubkey,
  isTurnActive = false,
  profiles,
}: {
  agent: ManagedAgent | null;
  blocks: ChatActivityRenderBlock[];
  identityPubkey?: string;
  isTurnActive?: boolean;
  profiles?: UserProfileLookup;
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
          isTurnActive={isTurnActive}
          key={renderBlock.id}
          profiles={profiles}
          suppressPromptMessage={renderBlock.suppressPromptMessage}
        />
      ))}
    </>
  );
}

function ChatActivityBlockView({
  agent,
  block,
  identityPubkey,
  isTurnActive,
  profiles,
  suppressPromptMessage,
}: {
  agent: ManagedAgent | null;
  block: TranscriptDisplayBlock;
  identityPubkey?: string;
  isTurnActive: boolean;
  profiles?: UserProfileLookup;
  suppressPromptMessage: boolean;
}) {
  if (block.kind === "single") {
    return (
      <ChatActivityItemView
        agent={agent}
        identityPubkey={identityPubkey}
        item={block.item}
        profiles={profiles}
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
          suppressPromptMessage={suppressPromptMessage}
        />
      ))}
      {!hasLiveActivityItem(block) ? (
        <LiveTurnMarker
          icon={liveTurnMarkerIcon(block)}
          startedAt={getTurnStartedAt(block)}
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
  suppressPromptMessage,
}: {
  agent: ManagedAgent | null;
  block: Extract<TranscriptDisplayBlock, { kind: "turn" }>;
  identityPubkey?: string;
  profiles?: UserProfileLookup;
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
  suppressPromptMessage,
}: {
  agent: ManagedAgent | null;
  identityPubkey?: string;
  profiles?: UserProfileLookup;
  segment: TranscriptTurnSegment;
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
        <PromptSetupMarker context={segment.context} setup={segment.setup} />
      </>
    );
  }

  if (segment.kind === "setup") {
    return <TurnSetupMarker items={segment.items} />;
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
      suppressPromptMessage={suppressPromptMessage}
    />
  );
}

function ChatActivityItemView({
  agent,
  identityPubkey,
  item,
  profiles,
  suppressPromptMessage,
}: {
  agent: ManagedAgent | null;
  identityPubkey?: string;
  item: TranscriptItem;
  profiles?: UserProfileLookup;
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
      />
    );
  }

  if (item.type === "tool") {
    return <ToolMarker item={item} />;
  }

  if (item.type === "thought") {
    return (
      <ActivityMarkerRow
        details={<Markdown compact content={item.text.trim() || " "} />}
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
            <Markdown
              compact
              content={item.text.trim() || "No plan details."}
            />
          )
        }
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
        icon={<FileText className="h-3.5 w-3.5" />}
        label={`Captured ${item.title.toLowerCase()}`}
        timestamp={item.timestamp}
        tone="muted"
      />
    );
  }

  return (
    <ActivityMarkerRow
      details={item.text ? <Markdown compact content={item.text} /> : null}
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
}: {
  agent: ManagedAgent | null;
  identityPubkey?: string;
  item: Extract<TranscriptItem, { type: "message" }>;
  profiles?: UserProfileLookup;
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

  return (
    <Message side={isUser ? "right" : "left"}>
      {!isUser ? (
        <MessageAvatar>
          <UserAvatar avatarUrl={avatarUrl} displayName={label} size="sm" />
        </MessageAvatar>
      ) : null}
      <MessageContent className={isUser ? "items-end" : "w-full max-w-full"}>
        <MessageHeader className={isUser ? "justify-end" : undefined}>
          <span className="truncate font-medium">{isUser ? "You" : label}</span>
        </MessageHeader>
        {isUser ? (
          <Bubble side="right">
            <Markdown
              className="min-w-0 [&_*]:text-primary-foreground [&_a]:text-primary-foreground [&_code]:bg-primary-foreground/15 [&_code]:text-primary-foreground"
              compact
              content={displayText || text || " "}
            />
          </Bubble>
        ) : (
          <Markdown
            className="w-full max-w-none text-sm leading-6"
            content={displayText || text || " "}
          />
        )}
      </MessageContent>
    </Message>
  );
}

function PromptSetupMarker({
  context,
  setup,
}: {
  context: Extract<TranscriptItem, { type: "metadata" }> | null;
  setup: Extract<TranscriptItem, { type: "lifecycle" }>[];
}) {
  return (
    <>
      <TurnSetupMarker items={setup} />
      {context ? (
        <ActivityMarkerRow
          details={<PromptSections sections={context.sections} />}
          icon={<FileText className="h-3.5 w-3.5" />}
          label="Captured prompt context"
          timestamp={context.timestamp}
          tone="muted"
        />
      ) : null}
    </>
  );
}

function TurnSetupMarker({
  items,
}: {
  items: Extract<TranscriptItem, { type: "lifecycle" }>[];
}) {
  if (items.length === 0) {
    return null;
  }

  return (
    <>
      {items.map((item) => (
        <LifecycleMarker item={item} key={item.id} />
      ))}
    </>
  );
}

function LifecycleMarker({
  item,
}: {
  item: Extract<TranscriptItem, { type: "lifecycle" }>;
}) {
  return (
    <ActivityMarkerRow
      details={
        item.text ? (
          <Markdown compact content={item.text} />
        ) : (
          <span>No additional details.</span>
        )
      }
      icon={lifecycleIcon(item)}
      label={item.title}
      timestamp={item.timestamp}
      tone={item.renderClass === "error" ? "danger" : "muted"}
    />
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
      icon={summaryIcon(summary)}
      label={summary.label}
      timestamp={summary.timestamp}
      tone="default"
    />
  );
}

function CompletedWorkMarker({ items }: { items: TranscriptItem[] }) {
  const [isOpen, setIsOpen] = React.useState(false);
  const label = completedWorkLabel(items);

  return (
    <Message className="px-0 py-1" side="left">
      <MessageContent className="w-full max-w-full">
        <button
          aria-expanded={isOpen}
          className="block w-full min-w-0 text-left"
          onClick={() => setIsOpen((current) => !current)}
          type="button"
        >
          <span className="flex min-w-0 items-center gap-1.5 text-foreground">
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
              {items.map((item) => (
                <CompletedWorkDetailRow item={item} key={item.id} />
              ))}
            </div>
          </div>
        ) : null}
      </MessageContent>
    </Message>
  );
}

function LiveTurnMarker({
  icon,
  startedAt,
}: {
  icon: React.ReactNode;
  startedAt: string | null;
}) {
  const elapsedSeconds = useElapsedSeconds(startedAt);

  return (
    <ActivityMarkerRow
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
          <span className="min-w-0 truncate text-foreground/80">
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
    return <Markdown compact content={item.text.trim() || " "} />;
  }
  if (item.type === "lifecycle") {
    return item.text ? <Markdown compact content={item.text} /> : null;
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
            <Markdown compact content={section.body.trim() || "No metadata."} />
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
