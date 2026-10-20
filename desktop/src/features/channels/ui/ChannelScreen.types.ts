import type { ReactNode } from "react";

import type {
  Channel,
  Identity,
  Profile,
  RelayEvent,
} from "@/shared/api/types";
import type { IdleAuxiliaryHeaderControls } from "./IdleAuxiliaryPanel";

export type ChannelScreenProps = {
  activeChannel: Channel | null;
  /**
   * When non-null, the main channel composer auto-submits once on mount after
   * loading the draft identified by this key. The route component clears the
   * `?autoSend` search param after the submit fires so back-navigation does
   * not re-trigger. Value must match the composer's `effectiveDraftKey`.
   */
  autoSendDraftKey: string | null;
  currentIdentity?: Identity;
  currentProfile?: Profile;
  idleAuxiliaryPanel?: ReactNode;
  idleAuxiliaryHeaderActions?: IdleAuxiliaryHeaderControls;
  idleAuxiliaryOverridesThread?: boolean;
  idleAuxiliaryTitle?: string;
  headerEndActions?: ReactNode;
  onAddFiles?: () => void;
  onCloseIdleAuxiliaryPanel?: () => void;
  onCloseForumPost: () => void;
  onSelectForumPost: (postId: string) => void;
  selectedForumPostId: string | null;
  targetForumReplyId: string | null;
  targetMessageEvents: RelayEvent[];
  targetMessageId: string | null;
  /** Exact clicked result id, retained after route target cleanup. */
  targetSearchMessageId?: string;
  /** Search text to highlight within the opened result message. */
  targetSearchQuery?: string;
};
