import type * as React from "react";

import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";

export type MessageComposerProps = {
  autoInviteNonMemberMentions?: boolean;
  channelId?: string | null;
  channelName: string;
  channelType?: ChannelType | null;
  containerClassName?: string;
  disabled?: boolean;
  draftKey?: string;
  editTarget?: {
    author: string;
    body: string;
    id: string;
    /**
     * NIP-92 imeta attachments on the original event, in tag order. Loaded
     * into the composer's pending-imeta state on edit-open so the user sees
     * them as removable thumbnails (just like the send path) and can add
     * more. The submit path emits a fresh full imeta tag set on the edit
     * event; the receiver overlays it.
     */
    imetaMedia?: ImetaMedia[];
  } | null;
  isSending?: boolean;
  mediaController?: MediaUploadController;
  onCancelEdit?: () => void;
  onCancelReply?: () => void;
  /**
   * Invoked when the user presses ↑ in an empty composer that is not already
   * in edit mode. The owner should locate the most recent message authored by
   * the current user within this composer's scope (main timeline, DM, or
   * thread) and enter edit mode for it. Return `true` if a target was found
   * and edit mode was entered, so the composer can swallow the keystroke;
   * return `false` to let the arrow key fall through normally.
   */
  onEditLastOwnMessage?: () => boolean;
  onEditSave?: (content: string, mediaTags?: string[][]) => Promise<void>;
  /**
   * Called synchronously at the start of `submitMessage`, before any awaits,
   * to capture context that must be stable throughout the async send pipeline.
   * Used by the thread-reply composer to capture the current reply target before
   * the mention-flow awaits can change navigation state.
   */
  onCaptureSendContext?: () => {
    parentEventId: string | null;
    threadHeadId: string | null;
  } | null;
  onSend: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    channelId?: string | null,
    threadContext?: {
      parentEventId: string | null;
      threadHeadId: string | null;
    } | null,
  ) => Promise<void>;
  /**
   * When set, the send button becomes a stop button that interrupts the
   * channel's working agent. Pass only while a turn is actually active.
   */
  onStopAgent?: (() => void) | null;
  placeholder?: string;
  profiles?: UserProfileLookup;
  replyTarget?: {
    author: string;
    body: string;
    id: string;
  } | null;
  showTopBorder?: boolean;
  toolbarControls?: {
    emoji?: boolean;
    formatting?: boolean;
    spoiler?: boolean;
  };
  toolbarExtraActions?: React.ReactNode;
  typingParentEventId?: string | null;
  typingRootEventId?: string | null;
};
