import type {
  Channel,
  Identity,
  Profile,
  RelayEvent,
} from "@/shared/api/types";

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
  onCloseForumPost: () => void;
  onSelectForumPost: (postId: string) => void;
  selectedForumPostId: string | null;
  targetForumReplyId: string | null;
  targetMessageEvents: RelayEvent[];
  targetMessageId: string | null;
  /**
   * Thread root requested by the navigation source (`?threadRootId`, or the
   * `?thread` panel param on deep links). Deciding input for top-level route
   * targets: present → open the thread panel at that root; absent → the
   * target is shown in the main timeline only.
   */
  targetThreadRootId: string | null;
};
