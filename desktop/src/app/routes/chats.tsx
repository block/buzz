import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { NO_PROJECT_SELECTION_ID } from "@/features/chats/lib/chatSetup";

type ChatsRouteSearch = {
  projectId?: string;
};

function validateChatsSearch(
  search: Record<string, unknown>,
): ChatsRouteSearch {
  const projectId = search.projectId;
  return {
    projectId:
      typeof projectId === "string" && projectId.trim().length > 0
        ? projectId
        : undefined,
  };
}

const LazyChatsScreen = React.lazy(async () => {
  const module = await import("@/features/chats/ui/ChatsScreen");
  return { default: module.ChatsScreen };
});

export const Route = createFileRoute("/chats")({
  validateSearch: validateChatsSearch,
  component: ChatsRoute,
});

function ChatsRoute() {
  const search = Route.useSearch();
  const initialProjectId =
    search.projectId === NO_PROJECT_SELECTION_ID ? null : search.projectId;

  return (
    <React.Suspense fallback={null}>
      <LazyChatsScreen initialProjectId={initialProjectId} />
    </React.Suspense>
  );
}
