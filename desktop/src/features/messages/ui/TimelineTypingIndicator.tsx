import { type ComponentProps, useEffect, useState } from "react";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { cn } from "@/shared/lib/cn";
import { BubbleAvatar } from "./BubbleAvatarScope";
import { TypingDots } from "./TypingDots";
import type { TypingIndicatorRow } from "./TypingIndicatorRow";
import "./MessageBubbleLayout.css";

/** One incoming bubble for everyone typing, inside the conversation's scroll flow. */
export function TimelineTypingIndicator(
  props: ComponentProps<typeof TypingIndicatorRow>,
) {
  const active = props.typingPubkeys.length > 0;
  const [lastTyping, setLastTyping] = useState(props.typingPubkeys);
  useEffect(() => {
    if (props.typingPubkeys.length > 0) setLastTyping(props.typingPubkeys);
  }, [props.typingPubkeys]);
  const pubkeys = active ? props.typingPubkeys : lastTyping;
  const labels = pubkeys.map((pubkey) => {
    const participant =
      props.channel?.participantPubkeys.findIndex(
        (key) => key.toLowerCase() === pubkey.toLowerCase(),
      ) ?? -1;
    return resolveUserLabel({
      pubkey,
      currentPubkey: props.currentPubkey,
      profiles: props.profiles,
      fallbackName: props.channel?.participants[participant],
      preferResolvedSelfLabel: true,
    });
  });
  const grouped = pubkeys.length > 1;

  return (
    <div
      className="relative flex h-12 shrink-0 items-start px-3 pt-px"
      data-testid="timeline-typing-slot"
    >
      <div
        className="absolute left-3 top-[9px] flex h-7 w-7 items-end justify-end"
        aria-hidden="true"
      >
        {active &&
          pubkeys.map((pubkey, index) => (
            <div
              key={pubkey}
              className={cn("shrink-0", grouped ? "absolute" : "relative")}
              style={
                grouped
                  ? {
                      left: (index * 8) / (pubkeys.length - 1),
                      top: (index * 8) / (pubkeys.length - 1),
                    }
                  : undefined
              }
              data-testid="message-typing-avatar"
            >
              <BubbleAvatar typingPubkey={pubkey}>
                <div
                  className={cn(grouped && "typing-avatar-stroke rounded-full")}
                >
                  <UserAvatar
                    avatarUrl={
                      props.profiles?.[pubkey.toLowerCase()]?.avatarUrl ?? null
                    }
                    displayName={labels[index]}
                    fallbackDelayMs={0}
                    className={cn("text-3xs", grouped ? "h-5 w-5" : "h-7 w-7")}
                    shape="circle"
                  />
                </div>
              </BubbleAvatar>
            </div>
          ))}
      </div>
      <div
        aria-hidden={!active}
        className="timeline-typing-transition ml-[38px]"
        data-active={active}
        data-testid="timeline-typing-transition"
      >
        <div
          className="message-bubble-anchor"
          data-testid="message-typing-indicator"
        >
          <div
            className="typing-dots-bubble flex h-9 w-14 items-center justify-center gap-1 rounded-[20px]"
            aria-hidden="true"
          >
            <TypingDots />
          </div>
        </div>
      </div>
      <span className="sr-only" role="status">
        {active ? `${labels.join(", ")} ${grouped ? "are" : "is"} typing` : ""}
      </span>
    </div>
  );
}
