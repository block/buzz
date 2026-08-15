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

// WebKitGTK (the engine under every Linux Tauri webview) crashes the UI
// process when a view transition is the first thing to demand accelerated
// compositing on a machine whose accelerated backing store could not be
// created (X11 sessions, dmabuf transport disabled or unavailable): the
// missing-store guard is a debug ASSERT that release builds compile out, so
// document.startViewTransition() segfaults instead of animating. Reproduced
// 100% outside Buzz on WebKitGTK 2.52 regardless of the dmabuf env vars —
// see #3488, #4142, and https://bugs.webkit.org/show_bug.cgi?id=321683.
//
// Keyed on the platform alone, deliberately: this guards a UI-process
// segfault, so it has to fail closed. Narrowing it to WebKitGTK would mean
// sniffing the user agent for the engine, and a UA test that quietly stops
// matching re-enables the crash with nothing pointing back at this line. On
// Tauri, Linux means WebKitGTK anyway, so the cost of over-matching is only a
// missing cross-fade in a Linux browser session — a crash is not a tradeoff
// worth taking for an animation.
export function shouldSkipCommunityViewTransition(): boolean {
  return isLinuxPlatform();
}

export async function runCommunityViewTransition(
  update: () => Promise<void> | void,
  options: { timeoutMs?: number } = {},
): Promise<void> {
  if (!document.startViewTransition || shouldSkipCommunityViewTransition()) {
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
