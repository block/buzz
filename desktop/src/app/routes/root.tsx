import { createRootRoute } from "@tanstack/react-router";

import { AppShell } from "@/app/AppShell";

export const Route = createRootRoute({
  component: AppShell,
});
