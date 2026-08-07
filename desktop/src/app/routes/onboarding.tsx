import { createFileRoute, redirect } from "@tanstack/react-router";

// Onboarding is rendered by startup gates before RouterProvider mounts. If a
// stale session-only onboarding URL reaches the normal app router, repair it
// instead of rendering onboarding without its required in-memory state.
export const Route = createFileRoute("/onboarding/$step")({
  beforeLoad: () => {
    throw redirect({ to: "/", replace: true });
  },
});
