import { createFileRoute } from "@tanstack/react-router";
import { IdentityHandoffInvitePage } from "@/features/invite/ui/InvitePage";
import * as React from "react";

const IDENTITY_HANDOFF_FRAGMENT_LENGTH = "code=v3.".length + 64;
const IDENTITY_HANDOFF_FRAGMENT = /^code=(v3\.[0-9a-f]{64})$/;

function captureIdentityHandoffCode(): string | null {
  if (typeof window === "undefined" || window.location.pathname !== "/invite") {
    return null;
  }

  const fragment = window.location.hash.startsWith("#")
    ? window.location.hash.slice(1)
    : window.location.hash;
  const hasQuery = window.location.search.length > 0;
  if (fragment || hasQuery) {
    window.history.replaceState(
      window.history.state,
      "",
      window.location.pathname,
    );
  }

  if (hasQuery) return null;
  if (fragment.length !== IDENTITY_HANDOFF_FRAGMENT_LENGTH) return null;
  return IDENTITY_HANDOFF_FRAGMENT.exec(fragment)?.[1] ?? null;
}

function scrubLateFragment(): void {
  if (window.location.pathname !== "/invite") return;
  if (!window.location.hash && !window.location.search) return;
  window.history.replaceState(
    window.history.state,
    "",
    window.location.pathname,
  );
}

// The route module is evaluated before React mounts. Capture and scrub the
// bounded fragment once so Strict Mode cannot reread it on a second render.
// The code remains only in this page process and disappears on reload.
const identityHandoffCode = captureIdentityHandoffCode();

export const Route = createFileRoute("/invite")({
  component: IdentityHandoffInvitePageRoute,
});

function IdentityHandoffInvitePageRoute() {
  React.useEffect(() => {
    window.addEventListener("hashchange", scrubLateFragment);
    window.addEventListener("popstate", scrubLateFragment);
    return () => {
      window.removeEventListener("hashchange", scrubLateFragment);
      window.removeEventListener("popstate", scrubLateFragment);
    };
  }, []);

  return <IdentityHandoffInvitePage code={identityHandoffCode} />;
}
