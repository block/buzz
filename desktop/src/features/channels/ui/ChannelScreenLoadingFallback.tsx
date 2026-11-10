import { HuddleStartingView } from "@/features/huddle/components/HuddleStartingView";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

export function ChannelScreenLoadingFallback({
  isHuddleTranscript,
}: {
  isHuddleTranscript: boolean;
}) {
  return isHuddleTranscript ? (
    <HuddleStartingView />
  ) : (
    <ViewLoadingFallback includeHeader kind="channel" />
  );
}
