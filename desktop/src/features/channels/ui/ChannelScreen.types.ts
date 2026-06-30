import type {
  Channel,
  Identity,
  Profile,
  RelayEvent,
} from "@/shared/api/types";

export type ChannelScreenProps = {
  activeChannel: Channel | null;
  currentIdentity?: Identity;
  currentProfile?: Profile;
  onCloseForumPost: () => void;
  onSelectForumPost: (postId: string) => void;
  selectedForumPostId: string | null;
  targetAgentConversationBackfillPending?: boolean;
  targetAgentConversationReplyId: string | null;
  targetForumReplyId: string | null;
  targetMessageEvents: RelayEvent[];
  targetMessageId: string | null;
};
