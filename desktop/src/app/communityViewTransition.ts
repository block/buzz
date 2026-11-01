import { isLinuxPlatform } from "../shared/lib/platform";

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
  // Linux WebKitGTK can SIGSEGV while creating the composited snapshot on
  // NVIDIA (buzz#3654), even with the safe-rendering environment overrides.
  // Skip only the native snapshot, not the navigation or target-ready barrier.
  // Chromium also advertises AppleWebKit, so exclude Chromium-family browsers.
  const isLinuxWebKit =
    isLinuxPlatform() &&
    /AppleWebKit/i.test(navigator.userAgent) &&
    !/(?:Chrome|Chromium|Edg|OPR)\//i.test(navigator.userAgent);
  let finish: (() => void) | undefined;
  const targetReady = new Promise<void>((resolve) => {
    finish = resolve;
  });
  finishPendingTransition?.();
  finishPendingTransition = finish ?? null;

  const timeout = window.setTimeout(
    () => finish?.(),
    options.timeoutMs ?? COMMUNITY_TRANSITION_TIMEOUT_MS,
  );

  try {
    const updateAndWaitForTarget = async () => {
      await update();
      await targetReady;
    };
    if (!document.startViewTransition || isLinuxWebKit) {
      await updateAndWaitForTarget();
    } else {
      const transition = document.startViewTransition(updateAndWaitForTarget);
      await transition.updateCallbackDone;
    }
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
