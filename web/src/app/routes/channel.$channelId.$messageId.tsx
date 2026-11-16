import { createFileRoute } from "@tanstack/react-router";
import { ConversationLinkPage } from "@/features/conversation-link/ui/ConversationLinkPage";

export const Route = createFileRoute("/channel/$channelId/$messageId")({
  component: MessageLinkRoute,
});

function MessageLinkRoute() {
  const { channelId, messageId } = Route.useParams();
  return <ConversationLinkPage channelId={channelId} messageId={messageId} />;
}
