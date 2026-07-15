import type * as React from "react";

import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";

export type MessageComposerProps = {
  channelId?: string | null;
  channelName: string;
  channelType?: ChannelType | null;
  containerClassName?: string;
  disabled?: boolean;
  draftKey?: string;
  autoSubmitDraftKey?: string | null;
  onAutoSubmitComplete?: () => void;
  editTarget?: {
    author: string;
    body: string;
    id: string;
    imetaMedia?: ImetaMedia[];
  } | null;
  isSending?: boolean;
  mediaController?: MediaUploadController;
  onCancelEdit?: () => void;
  onCancelReply?: () => void;
  onEditLastOwnMessage?: () => boolean;
  onEditSave?: (content: string, mediaTags?: string[][]) => Promise<void>;
  onCaptureSendContext?: () => {
    parentEventId: string | null;
    threadHeadId: string | null;
  } | null;
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
  replyTarget?: { author: string; body: string; id: string } | null;
  showTopBorder?: boolean;
  toolbarExtraActions?: React.ReactNode;
  typingParentEventId?: string | null;
  typingRootEventId?: string | null;
};
