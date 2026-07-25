const DEFAULT_IDLE_DELAY_MS = 1_500;
const POOF_GAIN = 0.34;

type TimerHandle = ReturnType<typeof globalThis.setTimeout>;

type PoofAudioPlayerOptions = {
  idleDelayMs?: number;
  setTimeout?: (callback: () => void, delayMs: number) => TimerHandle;
  clearTimeout?: (handle: TimerHandle) => void;
};

function disconnectQuietly(node: { disconnect: () => void } | null) {
  try {
    node?.disconnect();
  } catch {
    // The platform may already have disconnected or closed the node.
  }
}

export function createPoofAudioPlayer({
  idleDelayMs = DEFAULT_IDLE_DELAY_MS,
  setTimeout: schedule = (callback, delayMs) =>
    globalThis.setTimeout(callback, delayMs),
  clearTimeout: cancel = (handle) => globalThis.clearTimeout(handle),
}: PoofAudioPlayerOptions = {}) {
  let activePlaybacks = 0;
  let suspendTimer: TimerHandle | null = null;
  let suspendGeneration = 0;
  let pendingSuspend: Promise<void> | null = null;

  function cancelPendingSuspend() {
    suspendGeneration += 1;
    if (suspendTimer !== null) {
      cancel(suspendTimer);
      suspendTimer = null;
    }
  }

  function scheduleSuspend(context: AudioContext) {
    const generation = suspendGeneration + 1;
    suspendGeneration = generation;
    suspendTimer = schedule(() => {
      suspendTimer = null;
      if (
        generation !== suspendGeneration ||
        activePlaybacks !== 0 ||
        context.state !== "running"
      ) {
        return;
      }
      const request = context.suspend().then(
        () => {},
        () => {
          // Best-effort only: a closed or platform-rejected context is harmless.
        },
      );
      pendingSuspend = request;
      void request.finally(() => {
        if (pendingSuspend === request) pendingSuspend = null;
      });
    }, idleDelayMs);
  }

  function play(
    context: AudioContext,
    buffer: AudioBuffer,
    playFallback: () => void,
  ) {
    cancelPendingSuspend();

    let source: AudioBufferSourceNode | null = null;
    let gain: GainNode | null = null;
    try {
      source = context.createBufferSource();
      gain = context.createGain();
      source.buffer = buffer;
      gain.gain.value = POOF_GAIN;
      source.connect(gain);
      gain.connect(context.destination);
    } catch {
      disconnectQuietly(source);
      disconnectQuietly(gain);
      playFallback();
      return;
    }

    const connectedSource = source;
    const connectedGain = gain;

    activePlaybacks += 1;
    let cleanedUp = false;

    function cleanup() {
      if (cleanedUp) return;
      cleanedUp = true;
      connectedSource.onended = null;
      disconnectQuietly(connectedSource);
      disconnectQuietly(connectedGain);
      activePlaybacks = Math.max(0, activePlaybacks - 1);
      if (activePlaybacks === 0) {
        scheduleSuspend(context);
      }
    }

    connectedSource.onended = cleanup;

    function start() {
      try {
        connectedSource.start();
      } catch {
        cleanup();
        playFallback();
      }
    }

    function resumeThenStart() {
      if (context.state === "running") {
        start();
        return;
      }
      void context.resume().then(start, () => {
        cleanup();
        playFallback();
      });
    }

    const suspendInFlight = pendingSuspend;
    if (suspendInFlight) {
      void suspendInFlight.then(resumeThenStart);
    } else {
      resumeThenStart();
    }
  }

  return { play };
}
