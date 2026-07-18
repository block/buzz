import type * as React from "react";

import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";

type MessageComposerAudienceContext = {
  type: "thread";
  threadRootId: string;
  initialAgentPubkeys?: readonly string[];
};

export type MessageComposerProps = {
  audienceContext?: MessageComposerAudienceContext | null;
  channelId?: string | null;
  channelName: string;
  channelType?: ChannelType | null;
  containerClassName?: string;
  disabled?: boolean;
  draftKey?: string;
  /**
   * When provided, the composer fires `submitMessage` once on mount after
   * the draft matching this key has been loaded into the editor. This powers
   * the "Send message" confirm-dialog flow in the Drafts panel. The callback
   * `onAutoSubmitComplete` must clear the trigger (e.g. remove `?autoSend`
   * from the URL) — it is called synchronously before `submitMessage` fires
   * so the param is gone before any navigation the send might cause.
   *
   * Fires at most once per mount: a stable key value that persists across
   * re-renders does NOT re-fire.
   */
  autoSubmitDraftKey?: string | null;
  /** Called when the auto-submit fires so the parent can clear the trigger. */
  onAutoSubmitComplete?: () => void;
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
  onEditSave?: (
    content: string,
    mediaTags?: string[][],
    mentionPubkeys?: string[],
  ) => Promise<void>;
  /** Captures send context synchronously before awaits can change navigation. */
  onCaptureSendContext?: () => {
    parentEventId: string | null;
    threadHeadId: string | null;
  } | null;
  /** Resolves the channel required to prepare mentions before sending. */
  onPrepareSendChannel?: (pubkeys?: string[]) => Promise<string | null>;
  onPreparingMentionSendChange?: (isPreparing: boolean) => void;
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
  placeholder?: string;
  profiles?: UserProfileLookup;
  replyTarget?: {
    author: string;
    body: string;
    id: string;
  } | null;
  showTopBorder?: boolean;
  toolbarExtraActions?: React.ReactNode;
  typingParentEventId?: string | null;
  typingRootEventId?: string | null;
};
