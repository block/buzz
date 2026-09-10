import { LayoutGroup, motion, useReducedMotion } from "motion/react";
import {
  createContext,
  type ReactNode,
  useContext,
  useId,
  useMemo,
} from "react";
import type { TimelineMessage } from "@/features/messages/types";

const BubbleAvatarContext = createContext<{
  lastMessage?: TimelineMessage;
  typingPubkeys: string[];
}>({ typingPubkeys: [] });

/** Share the final sender's avatar between their last bubble and typing bubble. */
export function BubbleAvatarScope({
  children,
  lastMessage,
  typingPubkeys,
}: {
  children: ReactNode;
  lastMessage?: TimelineMessage;
  typingPubkeys: string[];
}) {
  const id = useId();
  const value = useMemo(
    () => ({ lastMessage, typingPubkeys }),
    [lastMessage, typingPubkeys],
  );
  return (
    <LayoutGroup id={id}>
      <BubbleAvatarContext.Provider value={value}>
        {children}
      </BubbleAvatarContext.Provider>
    </LayoutGroup>
  );
}

/** Only the final visible message can hand its avatar to live typing feedback. */
export function BubbleAvatar({
  children,
  messageId,
  typingPubkey,
}: {
  children: ReactNode;
  messageId?: string;
  typingPubkey?: string;
}) {
  const { lastMessage, typingPubkeys } = useContext(BubbleAvatarContext);
  const reducedMotion = useReducedMotion();
  const lastPubkey = lastMessage?.pubkey?.toLowerCase();
  const isLastMessage = messageId != null && messageId === lastMessage?.id;
  const shared =
    isLastMessage ||
    (typingPubkey != null && typingPubkey.toLowerCase() === lastPubkey);
  if (
    isLastMessage &&
    typingPubkeys.some((pubkey) => pubkey.toLowerCase() === lastPubkey)
  ) {
    return null;
  }
  return (
    <motion.div
      className="flex"
      layoutId={shared && lastMessage ? `avatar-${lastMessage.id}` : undefined}
      initial={false}
      transition={{
        // Match --motion-duration-fast and --motion-ease-standard.
        layout: { duration: reducedMotion ? 0 : 0.18, ease: [0.25, 1, 0.5, 1] },
      }}
      data-testid="bubble-avatar"
    >
      {children}
    </motion.div>
  );
}
