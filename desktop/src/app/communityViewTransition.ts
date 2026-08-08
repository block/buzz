import { isLinuxPlatform } from "@/shared/lib/platform";

const COMMUNITY_TRANSITION_TIMEOUT_MS = 5_000;

let finishPendingTransition: (() => void) | null = null;

export function completeCommunityViewTransition(): void {
  finishPendingTransition?.();
}

export function replaceCommunityDestinationRoute(
  channelId: string,
  history: { replace: (href: string) => void },
): void {
  history.replace(`/channels/${encodeURIComponent(channelId)}`);
}

export async function runCommunityViewTransition(
  update: () => Promise<void> | void,
  options: { timeoutMs?: number } = {},
): Promise<void> {
  // Linux WebKit (webkitgtk, as shipped by the buzz AppImage on distributions
  // like Linux Mint) hangs indefinitely on `transition.updateCallbackDone`
  // whenever a destructive community switch removes the currently painted
  // frame mid-transition — the reasoning is not the 5s timeout, which does
  // resolve `targetReady` and lets `update()` finish; the WebKit layer just
  // never settles the view-transition promise on frame invalidation. That
  // freezes the window and forces users to SIGKILL the app (see #3931).
  // The transition is purely cosmetic; on Linux we lose nothing by running
  // the update directly and skipping the browser's cross-fade.
  if (!document.startViewTransition || isLinuxPlatform()) {
    try {
      await update();
    } catch (error) {
      console.error("Community transition failed:", error);
    }
    return;
  }

  let finish: (() => void) | undefined;
  const targetReady = new Promise<void>((resolve) => {
    finish = resolve;
  });
  finishPendingTransition?.();
  finishPendingTransition = finish ?? null;

  const timeout = window.setTimeout(
    () => completeCommunityViewTransition(),
    options.timeoutMs ?? COMMUNITY_TRANSITION_TIMEOUT_MS,
  );

  try {
    const transition = document.startViewTransition(async () => {
      await update();
      await targetReady;
    });
    await transition.updateCallbackDone;
  } catch (error) {
    // Event handlers intentionally fire-and-forget community switches. Contain
    // navigation/apply failures here so rejection cannot escape React; update()
    // either leaves the current route intact or at the deliberate Home barrier.
    console.error("Community transition failed:", error);
  } finally {
    window.clearTimeout(timeout);
    if (finishPendingTransition === finish) {
      finishPendingTransition = null;
    }
  }
}
