import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { TranscriptItem } from "../agentSessionTypes";
import { ToolActivity } from "./ToolActivity";
import { TranscriptTimestamp } from "./TranscriptTimestamp";
import type {
  ActivityRenderClassItemProps,
  AgentTranscriptIdentityProps,
} from "./types";

export function MessageActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "message") {
    return null;
  }

  return (
    <MessageItem
      agentAvatarUrl={props.agentAvatarUrl}
      agentName={props.agentName}
      agentPubkey={props.agentPubkey}
      item={props.item}
      profiles={props.profiles}
    />
  );
}

function MessageItem({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  item,
  profiles,
}: AgentTranscriptIdentityProps & {
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
  const agentProfile = profiles?.[normalizePubkey(agentPubkey)] ?? null;
  const assistantLabel = resolveUserLabel({
    pubkey: agentPubkey,
    fallbackName: agentName,
    profiles,
    preferResolvedSelfLabel: true,
  });
  const assistantAvatarUrl = agentProfile?.avatarUrl ?? agentAvatarUrl;

  return (
    <div
      className={cn(
        "flex animate-in fade-in duration-200 motion-reduce:animate-none",
        isAssistant
          ? "flex-row px-0 py-1.5"
          : "flex-row items-start justify-end px-0 py-0.5",
      )}
      data-role={isAssistant ? "assistant-message" : "user-message"}
      data-testid={
        isAssistant ? "transcript-assistant-message" : "transcript-user-message"
      }
    >
      {!isAssistant ? (
        <UserAvatar
          avatarUrl={authorProfile?.avatarUrl ?? null}
          className="order-last ml-2 mt-1 shrink-0"
          displayName={authorLabel}
          size="xs"
        />
      ) : null}
      <div
        className={cn(
          "group relative flex min-w-0 flex-col gap-1",
          isAssistant ? "w-full items-start" : "max-w-[85%] items-end",
        )}
      >
        {isAssistant ? (
          <div className="mb-0.5 flex items-center gap-1.5 text-xs">
            <UserAvatar
              avatarUrl={assistantAvatarUrl}
              className="shrink-0"
              displayName={assistantLabel}
              size="xs"
              testId="transcript-assistant-avatar"
            />
            <span className="text-sm font-semibold text-foreground">
              {assistantLabel}
            </span>
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
              <Markdown content={text || " "} mediaInset tight />
              <TranscriptTimestamp timestamp={item.timestamp} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}
