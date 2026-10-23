export const MAX_PENDING_RELAY_FRAMES = 256;

export function createRelayInboundBuffer(
  handle: (message: unknown) => Promise<void>,
  onError: (error: unknown) => void,
) {
  let pending: unknown[] | undefined = [];
  let connected = false;
  let processing: Promise<void> | undefined;
  let rejectOverflow = (_error: Error) => {};
  const overflow = new Promise<never>((_resolve, reject) => {
    rejectOverflow = reject;
  });
  void overflow.catch(() => {});

  // One consumer must own complete IPC batches even after connecting. The
  // handler awaits each frame; concurrent batches let a later EOSE overtake
  // earlier EVENT frames and resolve history with only part of the result.
  function drain() {
    connected = true;
    processing ??= Promise.resolve().then(async () => {
      try {
        while (pending?.length) await handle(pending.shift());
      } catch (error: unknown) {
        if (pending !== undefined) {
          pending = undefined;
          onError(error);
        }
        throw error;
      } finally {
        processing = undefined;
      }
    });
    // receive() has no caller awaiting the pump. Its failure is reported once
    // through onError; connect's explicit drain() still receives the rejection.
    void processing.catch(() => {});
    return processing;
  }

  return {
    overflow,
    receive(message: unknown) {
      if (pending === undefined) return;
      if (pending.length >= MAX_PENDING_RELAY_FRAMES) {
        pending = undefined;
        const phase = connected ? "processing" : "connecting";
        const error = new Error(`Relay sent too many frames while ${phase}.`);
        rejectOverflow(error);
        onError(error);
        return;
      }
      pending.push(message);
      if (connected) void drain();
    },
    drain,
  };
}
