import { HuddleStartingView } from "@/features/huddle/components/HuddleStartingView";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

export function ChannelScreenLoadingFallback({
  includeHeader = true,
  isHuddleTranscript,
}: {
  includeHeader?: boolean;
  isHuddleTranscript: boolean;
}) {
  return isHuddleTranscript ? (
    <HuddleStartingView />
  ) : (
    <ViewLoadingFallback includeHeader={includeHeader} kind="channel" />
  );
}
