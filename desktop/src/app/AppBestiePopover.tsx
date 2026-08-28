import { BestieChatPopover } from "@/features/messages/ui/BestieChatPopover";
import { useFeatureEnabled } from "@/shared/features";

export function AppBestiePopover({ hidden }: { hidden: boolean }) {
  const bestieEnabled = useFeatureEnabled("bestie");
  return bestieEnabled ? <BestieChatPopover showTrigger={!hidden} /> : null;
}
