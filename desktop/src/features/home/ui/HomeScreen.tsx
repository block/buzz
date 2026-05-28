import { useHomeFeedQuery } from "@/features/home/hooks";
import { HomeView } from "@/features/home/ui/HomeView";

type HomeScreenProps = {
  availableChannelIds: ReadonlySet<string>;
  currentPubkey?: string;
  onOpenChannel: (channelId: string) => void;
  onOpenContext: (channelId: string, messageId: string) => void;
};

export function HomeScreen({
  availableChannelIds,
  currentPubkey,
  onOpenChannel,
  onOpenContext,
}: HomeScreenProps) {
  const homeFeedQuery = useHomeFeedQuery();

  return (
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
        onOpenChannel={onOpenChannel}
        onOpenContext={onOpenContext}
        onRefresh={() => {
          void homeFeedQuery.refetch();
        }}
      />
    </div>
  );
}
