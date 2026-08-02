type PlaybackSpeedPersistenceCallbacks = {
  setSpeed: (speed: number) => void;
  setError: (error: string | null) => void;
};

export function createPlaybackSpeedPersistence(
  persist: (speed: number) => Promise<void>,
  initialSpeed = 1,
) {
  let confirmedSpeed = initialSpeed;
  let desiredSpeed = initialSpeed;
  let error: string | null = null;
  let flushPromise: Promise<void> | null = null;
  let hasLocalIntent = false;
  const listeners = new Set<PlaybackSpeedPersistenceCallbacks>();

  const notify = () => {
    for (const listener of listeners) {
      listener.setSpeed(desiredSpeed);
      listener.setError(error);
    }
  };

  const flush = async () => {
    while (desiredSpeed !== confirmedSpeed) {
      const target = desiredSpeed;
      try {
        await persist(target);
        confirmedSpeed = target;
      } catch (cause) {
        if (desiredSpeed === target) {
          desiredSpeed = confirmedSpeed;
          error = String(cause);
          notify();
          return;
        }
      }
    }
  };

  const ensureFlush = () => {
    if (flushPromise) return;
    const currentFlush = flush().finally(() => {
      if (flushPromise !== currentFlush) return;
      flushPromise = null;
      if (desiredSpeed !== confirmedSpeed) ensureFlush();
    });
    flushPromise = currentFlush;
  };

  return {
    applyLoaded(savedSpeed: number) {
      if (hasLocalIntent) return;
      confirmedSpeed = savedSpeed;
      desiredSpeed = savedSpeed;
      notify();
    },
    commit(nextSpeed: number) {
      hasLocalIntent = true;
      desiredSpeed = nextSpeed;
      error = null;
      notify();
      ensureFlush();
    },
    subscribe(callbacks: PlaybackSpeedPersistenceCallbacks) {
      listeners.add(callbacks);
      callbacks.setSpeed(desiredSpeed);
      callbacks.setError(error);
      return () => listeners.delete(callbacks);
    },
  };
}
