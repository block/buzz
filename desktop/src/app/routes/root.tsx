import { createRootRoute } from "@tanstack/react-router";

import { AppShell } from "@/app/AppShell";
import { HuddlePresenceProvider } from "@/features/huddle/HuddlePresenceContext";
import { UserStatusLookupProvider } from "@/features/user-status/UserStatusLookupContext";

function RootRoute() {
  return (
    <HuddlePresenceProvider>
      <UserStatusLookupProvider>
        <AppShell />
      </UserStatusLookupProvider>
    </HuddlePresenceProvider>
  );
}

export const Route = createRootRoute({
  component: RootRoute,
});
