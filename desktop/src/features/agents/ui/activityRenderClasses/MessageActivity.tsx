import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { Markdown } from "@/shared/ui/markdown";
import { useAgentSessionTranscriptVariant } from "../agentSessionTranscriptContext";
import { formatTranscriptTimestampTitle } from "../agentSessionUtils";
import type { TranscriptItem } from "../agentSessionTypes";
import { ToolActivity } from "./ToolActivity";
import { TranscriptTimestamp } from "./TranscriptTimestamp";
import type { ActivityRenderClassItemProps } from "./types";
import { UserMessageBubble } from "./UserMessageBubble";

export function MessageActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "message") {
    return null;
  }

  return <MessageItem item={props.item} profiles={props.profiles} />;
}

function MessageItem({
  item,
  profiles,
}: {
  item: Extract<TranscriptItem, { type: "message" }>;
  profiles?: UserProfileLookup;
}) {
  const variant = useAgentSessionTranscriptVariant();
  const isCompactPreview = variant === "compactPreview";
  const isAssistant = item.role === "assistant";
  const text = item.text.trim();
  const messageLink = getTranscriptMessageLink(item);

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
        <div
          className={
            isCompactPreview
              ? "w-full min-w-0 text-xs leading-4"
              : "w-full min-w-0 text-sm"
          }
          title={formatTranscriptTimestampTitle(item.timestamp)}
        >
          <Markdown
            className={isCompactPreview ? "text-xs leading-4" : "leading-5"}
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
