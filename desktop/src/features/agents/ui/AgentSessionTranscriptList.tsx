import * as React from "react";
import { Bot, Brain, ChevronDown, Radio, TerminalSquare } from "lucide-react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { TranscriptItem } from "./agentSessionTypes";
import { ToolItem } from "./AgentSessionToolItem";
import { formatTranscriptTime } from "./agentSessionUtils";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function AgentSessionTranscriptList({
  agentName,
  emptyDescription,
  items,
  profiles,
}: {
  agentName: string;
  emptyDescription: string;
  items: TranscriptItem[];
  profiles?: UserProfileLookup;
}) {
  if (items.length === 0) {
    return (
      <div className="flex min-h-56 flex-col items-center justify-center px-6 py-10 text-center">
        <Radio className="mx-auto h-4 w-4 text-muted-foreground" />
        <p className="mt-3 text-sm font-medium">No ACP activity yet</p>
        <p className="mt-1 text-sm text-muted-foreground">{emptyDescription}</p>
      </div>
    );
  }

  return (
    <div
      aria-label="Live ACP transcript"
      aria-live="polite"
      className="w-full py-1"
      role="log"
    >
      {items.map((item) => (
        <div className="mt-4 first:mt-0" key={item.id}>
          <TranscriptItemView
            agentName={agentName}
            item={item}
            profiles={profiles}
          />
        </div>
      ))}
    </div>
  );
}

const TranscriptItemView = React.memo(function TranscriptItemView({
  agentName,
  item,
  profiles,
}: {
  agentName: string;
  item: TranscriptItem;
  profiles?: UserProfileLookup;
}) {
  if (item.type === "message") {
    return (
      <MessageItem agentName={agentName} item={item} profiles={profiles} />
    );
  }
  if (item.type === "tool") {
    return <ToolItem item={item} />;
  }
  if (item.type === "thought") {
    return <ThoughtItem item={item} />;
  }
  if (item.type === "metadata") {
    return <MetadataItem item={item} />;
  }
  return <LifecycleItem item={item} />;
});

function MessageItem({
  agentName,
  item,
  profiles,
}: {
  agentName: string;
  item: Extract<TranscriptItem, { type: "message" }>;
  profiles?: UserProfileLookup;
}) {
  const isAssistant = item.role === "assistant";
  const text = item.text.trim();
  const authorProfile = item.authorPubkey
    ? profiles?.[item.authorPubkey.toLowerCase()]
    : null;
  const authorLabel = item.authorPubkey
    ? resolveUserLabel({
        pubkey: item.authorPubkey,
        fallbackName: item.title,
        profiles,
      })
    : item.title || "User";

  return (
    <div
      className={cn(
        "flex flex-row px-1 py-1 animate-in fade-in duration-200 motion-reduce:animate-none",
      )}
      data-role={isAssistant ? "assistant-message" : "user-message"}
    >
      {!isAssistant ? (
        <UserAvatar
          avatarUrl={authorProfile?.avatarUrl ?? null}
          className="mr-2 mt-1 h-5 w-5 shrink-0 text-[8px]"
          displayName={authorLabel}
          size="xs"
        />
      ) : null}
      <div
        className={cn(
          "group relative min-w-0 flex flex-col items-start gap-1",
          isAssistant ? "w-full" : "max-w-[85%]",
        )}
      >
        {isAssistant ? (
          <div className="mb-0.5 flex items-center gap-1 text-xs">
            <span className="flex h-5 w-5 items-center justify-center">
              <Bot className="h-4 w-4 text-muted-foreground" />
            </span>
            <span className="font-normal text-foreground">{agentName}</span>
            <TranscriptTimestamp timestamp={item.timestamp} />
          </div>
        ) : null}
        <div
          className={cn(
            "w-full min-w-0 text-sm leading-relaxed",
            !isAssistant && "rounded-2xl bg-muted p-3 text-foreground",
          )}
        >
          {isAssistant ? (
            <Markdown compact content={text || " "} />
          ) : (
            <>
              <p className="whitespace-pre-wrap break-words">{text}</p>
              <TranscriptTimestamp timestamp={item.timestamp} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function ThoughtItem({
  item,
}: {
  item: Extract<TranscriptItem, { type: "thought" }>;
}) {
  return (
    <details className="group not-prose w-full px-1">
      <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 py-px text-muted-foreground">
        <Brain className="h-4 w-4" />
        <span className="truncate text-sm font-medium">{item.title}</span>
        <TranscriptTimestamp timestamp={item.timestamp} />
        <ChevronDown className="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
      </summary>
      <div className="py-2 pl-5 text-sm leading-6 text-muted-foreground">
        <Markdown compact content={item.text.trim() || " "} />
      </div>
    </details>
  );
}

function MetadataItem({
  item,
}: {
  item: Extract<TranscriptItem, { type: "metadata" }>;
}) {
  return (
    <details className="group not-prose w-full px-1">
      <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 py-px text-muted-foreground">
        <TerminalSquare className="h-4 w-4" />
        <span className="truncate text-sm font-medium">{item.title}</span>
        <span className="shrink-0 text-xs">
          {item.sections.length} section{item.sections.length === 1 ? "" : "s"}
        </span>
        <TranscriptTimestamp timestamp={item.timestamp} />
        <ChevronDown className="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
      </summary>
      <div className="space-y-3 py-2 pl-5">
        {item.sections.map((section) => (
          <details
            className="group/section"
            key={`${section.title}:${section.body.slice(0, 48)}`}
          >
            <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 text-xs font-medium text-foreground/80">
              <span className="truncate">{section.title}</span>
              <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-open/section:rotate-180" />
            </summary>
            <pre className="mt-2 max-h-56 overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted/50 px-3 py-2 font-mono text-[11px] leading-5 text-muted-foreground">
              {section.body.trim() || "No metadata."}
            </pre>
          </details>
        ))}
      </div>
    </details>
  );
}

function LifecycleItem({
  item,
}: {
  item: Extract<TranscriptItem, { type: "lifecycle" }>;
}) {
  const isError = item.title.toLowerCase().includes("error");
  return (
    <div
      className={cn(
        "flex items-center justify-start gap-1.5 px-1 py-2 text-left text-xs",
        isError ? "text-destructive" : "text-muted-foreground",
      )}
    >
      <span className="font-medium">{item.title}</span>
      {item.text ? <span> - {item.text}</span> : null}
      <TranscriptTimestamp timestamp={item.timestamp} />
    </div>
  );
}

const fullDateTimeFormat = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  second: "2-digit",
});

function TranscriptTimestamp({ timestamp }: { timestamp: string }) {
  const formatted = formatTranscriptTime(timestamp);
  if (!formatted) return null;
  const date = new Date(timestamp);
  const fullDateTime = Number.isNaN(date.getTime())
    ? timestamp
    : fullDateTimeFormat.format(date);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="shrink-0 cursor-default text-[11px] text-muted-foreground/60">
          {formatted}
        </span>
      </TooltipTrigger>
      <TooltipContent side="top">{fullDateTime}</TooltipContent>
    </Tooltip>
  );
}
