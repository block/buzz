import { FibreInboxView } from "@/features/home/ui/fibre/FibreInboxView";

type HomeScreenProps = {
  currentPubkey?: string;
  onOpenContext: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
};

export function HomeScreen({ currentPubkey, onOpenContext }: HomeScreenProps) {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <FibreInboxView
        currentPubkey={currentPubkey}
        onOpenContext={onOpenContext}
      />
    </div>
  );
}
