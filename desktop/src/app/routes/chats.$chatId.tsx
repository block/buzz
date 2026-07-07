import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

const LazyChatsScreen = React.lazy(async () => {
  const module = await import("@/features/chats/ui/ChatsScreen");
  return { default: module.ChatsScreen };
});

export const Route = createFileRoute("/chats/$chatId")({
  component: ChatRoute,
});

function ChatRoute() {
  const { chatId } = Route.useParams();

  return (
    <React.Suspense fallback={null}>
      <LazyChatsScreen selectedChatId={chatId} />
    </React.Suspense>
  );
}
