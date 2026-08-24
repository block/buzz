import { deriveShellRoute } from "@/app/AppShell.helpers";

const HOME_DETAIL_SEARCH_KEYS = [
  "item",
  "profile",
  "profileTab",
  "profileView",
] as const;

type RouteLocation = {
  pathname: string;
  search?: Record<string, unknown>;
};

function hasHomeDetailSelection(search: Record<string, unknown> | undefined) {
  if (!search) return false;
  return HOME_DETAIL_SEARCH_KEYS.some((key) => {
    const value = search[key];
    return typeof value === "string" && value.length > 0;
  });
}

export function shouldCmdWCloseWindowForRoute(location: RouteLocation) {
  const route = deriveShellRoute(location.pathname);
  if (route.selectedView === "channel" && route.selectedChannelId) {
    return false;
  }

  if (location.pathname === "/messages/new") {
    return false;
  }

  if (location.pathname === "/" && hasHomeDetailSelection(location.search)) {
    return false;
  }

  return true;
}
