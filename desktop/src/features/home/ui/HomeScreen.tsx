import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { HomeView } from "@/features/home/ui/HomeView";

type HomeScreenProps = {
  availableChannelIds: ReadonlySet<string>;
  currentPubkey?: string;
  onOpenContext: (channelId: string, messageId: string) => void;
};

export function HomeScreen({
  availableChannelIds,
  currentPubkey,
  onOpenContext,
}: HomeScreenProps) {
  const homeFeedQuery = useHomeFeedQuery();

  return (
    <>
      <ChatHeader
        description="Personalized activity feed for mentions, reminders, channel activity, and agent work."
        mode="home"
        overlaysContent
        title="Home"
      />

      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <HomeView
          availableChannelIds={availableChannelIds}
          currentPubkey={currentPubkey}
          errorMessage={
            homeFeedQuery.error instanceof Error
              ? homeFeedQuery.error.message
              : undefined
          }
          feed={homeFeedQuery.data}
          isLoading={homeFeedQuery.isLoading}
          onOpenContext={onOpenContext}
          onRefresh={() => {
            void homeFeedQuery.refetch();
          }}
        />
      </div>
    </>
  );
}
