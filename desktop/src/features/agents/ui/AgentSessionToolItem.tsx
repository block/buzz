import * as React from "react";
import { ArrowUpRight, ChevronDown, Wrench } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import type { Channel, UserProfileSummary } from "@/shared/api/types";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { cn } from "@/shared/lib/cn";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { TranscriptItem } from "./agentSessionTypes";
import {
  formatToolTitle,
  getBuzzToolInfo,
  getToolStatusDisplay,
} from "./agentSessionToolCatalog";
import {
  asRecord,
  formatCodeValue,
  formatDuration,
  formatTranscriptTime,
  getResultArray,
  getToolString,
  getToolStringList,
  shortenMiddle,
} from "./agentSessionUtils";

export function ToolItem({
  item,
}: {
  item: Extract<TranscriptItem, { type: "tool" }>;
}) {
  const [isExpanded, setIsExpanded] = React.useState(false);
  const status = getToolStatusDisplay(item.status, item.isError);
  const hasArgs = Object.keys(item.args).length > 0;
  const hasResult = item.result.trim().length > 0;
  const canonicalToolName = item.buzzToolName ?? item.toolName;
  const buzzTool = getBuzzToolInfo(canonicalToolName);
  const ToolIcon = buzzTool?.icon ?? Wrench;
  const showStatus = status.state !== "output-available";
  const toolTitle = formatToolTitle(canonicalToolName, item.title);
  const handleToggle = React.useCallback(
    (event: React.SyntheticEvent<HTMLDetailsElement>) => {
      setIsExpanded(event.currentTarget.open);
    },
    [],
  );

  return (
    <div className="not-prose w-full px-1">
      <details
        className="group w-full"
        onToggle={handleToggle}
        open={isExpanded}
      >
        <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 py-px">
          {ToolIcon ? (
            <ToolIcon
              className={cn(
                "h-4 w-4 shrink-0",
                buzzTool ? "text-primary" : "text-muted-foreground",
              )}
            />
          ) : null}
          <span className="min-w-0 truncate text-sm font-medium">
            {toolTitle}
          </span>
          {buzzTool ? (
            <BuzzToolInlineAction args={item.args} result={item.result} />
          ) : null}
          {showStatus ? (
            <span className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
              <status.Icon
                className={cn(
                  "h-4 w-4",
                  item.status === "executing" && "animate-pulse",
                )}
              />
              {status.label}
            </span>
          ) : null}
          <ToolTimestamp item={item} />
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180" />
        </summary>

        <ToolDetailBlocks
          args={item.args}
          description={buzzTool?.label}
          hasArgs={hasArgs}
          hasResult={hasResult}
          isError={item.isError}
          result={item.result}
        />
      </details>
    </div>
  );
}

function ToolDetailBlocks({
  args,
  description,
  hasArgs,
  hasResult,
  isError,
  result,
}: {
  args: Record<string, unknown>;
  description?: string;
  hasArgs: boolean;
  hasResult: boolean;
  isError: boolean;
  result: string;
}) {
  return (
    <div className="space-y-4 py-2 pl-5 text-popover-foreground outline-hidden">
      {description ? (
        <p className="max-w-2xl text-xs leading-5 text-muted-foreground">
          {description}
        </p>
      ) : null}
      {hasArgs ? (
        <ToolCodeBlock
          label="Parameters"
          tone="muted"
          value={JSON.stringify(args, null, 2)}
        />
      ) : null}
      {hasResult ? (
        <ToolCodeBlock
          label={isError ? "Error" : "Result"}
          tone={isError ? "error" : "muted"}
          value={result}
        />
      ) : null}
      {!hasArgs && !hasResult ? (
        <p className="text-sm text-muted-foreground/80">
          Waiting for tool details.
        </p>
      ) : null}
    </div>
  );
}

function ToolCodeBlock({
  label,
  tone,
  value,
}: {
  label: string;
  tone: "muted" | "error";
  value: string;
}) {
  return (
    <div className="space-y-2 overflow-hidden">
      <h4 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </h4>
      <pre
        className={cn(
          "max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md px-3 py-2 font-mono text-xs leading-5",
          tone === "error"
            ? "bg-destructive/10 text-destructive"
            : "bg-muted/50 text-foreground",
        )}
      >
        {formatCodeValue(value)}
      </pre>
    </div>
  );
}

const toolFullDateTimeFormat = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  second: "2-digit",
});

function ToolTimestamp({
  item,
}: {
  item: Extract<TranscriptItem, { type: "tool" }>;
}) {
  const time = formatTranscriptTime(item.timestamp);
  if (!time) return null;
  const duration =
    item.startedAt && item.completedAt
      ? formatDuration(item.startedAt, item.completedAt)
      : null;
  const date = new Date(item.timestamp);
  const fullDateTime = Number.isNaN(date.getTime())
    ? item.timestamp
    : toolFullDateTimeFormat.format(date);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="shrink-0 cursor-default text-2xs text-muted-foreground/60">
          {time}
          {duration ? ` · ${duration}` : null}
        </span>
      </TooltipTrigger>
      <TooltipContent side="top">{fullDateTime}</TooltipContent>
    </Tooltip>
  );
}

function BuzzToolInlineAction({
  args,
  result,
}: {
  args: Record<string, unknown>;
  result: string;
}) {
  const { channels } = useChannelNavigation();
  const { goChannel } = useAppNavigation();
  const resultValue = React.useMemo(
    () => parseToolResultValue(result),
    [result],
  );
  const resultRecord = asRecord(resultValue);
  const channelId =
    getToolString(args, ["channel_id", "channelId"]) ??
    getToolString(resultRecord, ["channel_id", "channelId"]);
  const pubkeys = React.useMemo(
    () => getToolStringList(args, ["pubkeys", "pubkey"]),
    [args],
  );
  const profilesQuery = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;
  const openChannel = React.useCallback(
    (messageId?: string) => {
      if (!channelId) return;
      void goChannel(channelId, messageId ? { messageId } : undefined);
    },
    [channelId, goChannel],
  );
  const action = React.useMemo(
    () =>
      getBuzzToolInlineAction({
        args,
        channelId,
        channels,
        openChannel,
        profiles,
        resultValue,
      }),
    [args, channelId, channels, openChannel, profiles, resultValue],
  );

  if (!action) {
    return null;
  }

  if (action.onClick) {
    return (
      <button
        className="inline-flex max-w-[14rem] shrink min-w-0 items-center gap-1 rounded-full border border-primary/20 bg-primary/[0.05] px-1.5 py-0.5 text-2xs font-normal leading-none text-primary/90 transition-colors hover:border-primary/35 hover:bg-primary/10 hover:text-primary"
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          action.onClick?.();
        }}
        title={action.title}
        type="button"
      >
        {action.avatar}
        <span className="shrink-0">{action.label}</span>
        <span className="truncate">{action.value}</span>
        <ArrowUpRight className="h-4 w-4 shrink-0" />
      </button>
    );
  }

  return (
    <span
      className="inline-flex max-w-[14rem] shrink min-w-0 items-center gap-1 rounded-full border border-border/60 bg-muted/40 px-1.5 py-0.5 text-2xs font-normal leading-none text-muted-foreground"
      title={action.title}
    >
      {action.avatar}
      <span className="shrink-0">{action.label}</span>
      <span className="truncate">{action.value}</span>
    </span>
  );
}

type BuzzToolInlineActionModel = {
  avatar?: React.ReactNode;
  label: string;
  value: string;
  title: string;
  onClick?: () => void;
};

function getBuzzToolInlineAction({
  args,
  channelId,
  channels,
  openChannel,
  profiles,
  resultValue,
}: {
  args: Record<string, unknown>;
  channelId: string | null;
  channels: Channel[];
  openChannel: (messageId?: string) => void;
  profiles: Record<string, UserProfileSummary> | undefined;
  resultValue: unknown;
}): BuzzToolInlineActionModel | null {
  const resultRecord = asRecord(resultValue);
  const eventId =
    getToolString(args, ["event_id", "eventId"]) ??
    getToolString(resultRecord, ["event_id", "eventId", "id"]);

  if (eventId && channelId) {
    return {
      label: resultRecord.accepted === true ? "posted" : "event",
      onClick: () => openChannel(eventId),
      title: eventId,
      value: getChannelChipLabel(channels, channelId),
    };
  }

  const messages = getResultArray(resultValue, resultRecord, "messages");
  if (messages) {
    return {
      label: "read",
      onClick: channelId ? () => openChannel() : undefined,
      title: `${messages.length} messages`,
      value: `${messages.length} message${messages.length === 1 ? "" : "s"}`,
    };
  }

  if (channelId) {
    return {
      label: "channel",
      onClick: () => openChannel(),
      title: channelId,
      value: getChannelChipLabel(channels, channelId),
    };
  }

  const workflowId =
    getToolString(args, ["workflow_id", "workflowId"]) ??
    getToolString(resultRecord, ["workflow_id", "workflowId"]);
  if (workflowId) {
    return {
      label: "workflow",
      title: workflowId,
      value: shortenMiddle(workflowId, 26),
    };
  }

  const pubkeys = getToolStringList(args, ["pubkeys", "pubkey"]);
  if (pubkeys.length > 0) {
    if (pubkeys.length === 1) {
      const pk = pubkeys[0];
      const displayName = resolveUserLabel({ pubkey: pk, profiles });
      const profile = profiles?.[pk.toLowerCase()];
      return {
        avatar: (
          <UserAvatar
            avatarUrl={profile?.avatarUrl ?? null}
            className="shrink-0"
            displayName={displayName}
            size="xs"
          />
        ),
        label: "user",
        title: pk,
        value: displayName,
      };
    }
    return {
      label: "users",
      title: pubkeys
        .map((pk) => resolveUserLabel({ pubkey: pk, profiles }))
        .join(", "),
      value: `${pubkeys.length} users`,
    };
  }

  const query = getToolString(args, ["query"]);
  if (query) {
    return {
      label: "query",
      title: query,
      value: shortenMiddle(query, 30),
    };
  }

  if (typeof resultRecord.accepted === "boolean") {
    return {
      label: "relay",
      title: resultRecord.accepted ? "accepted" : "rejected",
      value: resultRecord.accepted ? "accepted" : "rejected",
    };
  }

  return null;
}

function parseToolResultValue(result: string): unknown {
  const trimmed = result.trim();
  if (!trimmed) return null;

  try {
    const parsed = JSON.parse(trimmed);
    if (typeof parsed !== "string") return parsed;
    try {
      return JSON.parse(parsed);
    } catch {
      return parsed;
    }
  } catch {
    return null;
  }
}

function getChannelChipLabel(channels: Channel[], channelId: string) {
  const channel = channels.find((candidate) => candidate.id === channelId);
  return channel ? `#${channel.name}` : `#${shortenMiddle(channelId, 22)}`;
}
