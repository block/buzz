import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { cn } from "@/shared/lib/cn";
import { useAgentSessionTranscriptVariant } from "../agentSessionTranscriptContext";
import type { TranscriptItem } from "../agentSessionTypes";
import { ToolActivity } from "./ToolActivity";
import { TranscriptTimestamp } from "./TranscriptTimestamp";
import type {
  ActivityRenderClassItemProps,
  AgentTranscriptIdentityProps,
} from "./types";
import { UserMessageBubble } from "./UserMessageBubble";

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
  const variant = useAgentSessionTranscriptVariant();
  const isCompactPreview = variant === "compactPreview";
  const isAssistant = item.role === "assistant";
  const text = item.text.trim();
  const messageLink = getTranscriptMessageLink(item);
  const agentProfile = profiles?.[normalizePubkey(agentPubkey)] ?? null;
  const assistantLabel = resolveUserLabel({
    pubkey: agentPubkey,
    fallbackName: agentName,
    profiles,
    preferResolvedSelfLabel: true,
  });
  const assistantAvatarUrl = agentProfile?.avatarUrl ?? agentAvatarUrl;

  if (!isAssistant) {
    return (
      <UserMessageBubble
        footer={
          <TranscriptTimestamp
            messageLink={messageLink}
            timestamp={item.timestamp}
          />
        }
        item={item}
        profiles={profiles}
      />
    );
  }

  return (
    <div
      className="flex flex-row animate-in fade-in duration-200 motion-reduce:animate-none"
      data-role="assistant-message"
      data-testid="transcript-assistant-message"
    >
      <div className="group relative flex w-full min-w-0 flex-col items-start gap-1">
        {isCompactPreview ? null : (
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
            <TranscriptTimestamp
              messageLink={messageLink}
              timestamp={item.timestamp}
            />
          </div>
        )}
        <div
          className={cn(
            "w-full min-w-0 text-sm",
            isCompactPreview && "text-xs leading-4",
          )}
        >
          <Markdown
            className={isCompactPreview ? "text-xs leading-4" : undefined}
            content={text || " "}
          />
        </div>
      </div>
    </div>
  );
}

function getTranscriptMessageLink(
  item: Extract<TranscriptItem, { type: "message" }>,
) {
  if (!item.channelId || !item.messageId) return null;
  return {
    channelId: item.channelId,
    messageId: item.messageId,
  };
}
