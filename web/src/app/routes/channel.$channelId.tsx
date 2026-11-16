import { createFileRoute } from "@tanstack/react-router";
import { ConversationLinkPage } from "@/features/conversation-link/ui/ConversationLinkPage";

export const Route = createFileRoute("/channel/$channelId")({
  component: ChannelLinkRoute,
});

function ChannelLinkRoute() {
  const { channelId } = Route.useParams();
  return <ConversationLinkPage channelId={channelId} />;
}
