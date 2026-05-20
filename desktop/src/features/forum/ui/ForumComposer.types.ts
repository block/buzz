import type * as React from "react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelMember } from "@/shared/api/types";

export type ForumComposerProps = {
  channelId?: string | null;
  /** Override mention source when no channel is available (e.g. Pulse). */
  members?: ChannelMember[];
  className?: string;
  placeholder: string;
  disabled?: boolean;
  header?: React.ReactNode;
  isSending?: boolean;
  onCancel?: () => void;
  onSubmit: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => undefined | Promise<unknown>;
  /** Render as a single-line composer until the user focuses it. */
  compact?: boolean;
  /** When true, autocomplete renders below the input (for top-of-view composers). */
  autocompleteBelow?: boolean;
  profiles?: UserProfileLookup;
};
