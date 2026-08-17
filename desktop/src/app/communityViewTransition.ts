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
  if (!document.startViewTransition) {
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
