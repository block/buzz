import { PulseView } from "@/features/pulse/ui/PulseView";
import { useIdentityQuery } from "@/shared/api/hooks";

export function PulseScreen() {
  const identityQuery = useIdentityQuery();

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <PulseView currentPubkey={identityQuery.data?.pubkey} />
      </div>
    </div>
  );
}
