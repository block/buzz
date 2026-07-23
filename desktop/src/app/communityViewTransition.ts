const COMMUNITY_TRANSITION_TIMEOUT_MS = 5_000;

let finishPendingTransition: (() => void) | null = null;

export function completeCommunityViewTransition(): void {
  finishPendingTransition?.();
}

export async function runCommunityViewTransition(
  update: () => Promise<void> | void,
): Promise<void> {
  if (!document.startViewTransition) {
    await update();
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
    COMMUNITY_TRANSITION_TIMEOUT_MS,
  );
  const transition = document.startViewTransition(async () => {
    await update();
    await targetReady;
  });

  try {
    await transition.updateCallbackDone;
  } finally {
    window.clearTimeout(timeout);
    if (finishPendingTransition === finish) {
      finishPendingTransition = null;
    }
  }
}
