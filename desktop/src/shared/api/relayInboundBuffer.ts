export const MAX_PENDING_RELAY_FRAMES = 256;

export function createRelayInboundBuffer(
  handle: (message: unknown) => Promise<void>,
  onError: (error: unknown) => void,
) {
  let pending: unknown[] | null | undefined = [];
  let rejectOverflow = (_error: Error) => {};
  const overflow = new Promise<never>((_resolve, reject) => {
    rejectOverflow = reject;
  });
  void overflow.catch(() => {});

  return {
    overflow,
    receive(message: unknown) {
      if (pending === null) {
        void handle(message).catch(onError);
        return;
      }
      if (pending === undefined) return;
      if (pending.length >= MAX_PENDING_RELAY_FRAMES) {
        pending = undefined;
        const error = new Error("Relay sent too many frames while connecting.");
        rejectOverflow(error);
        onError(error);
        return;
      }
      pending.push(message);
    },
    async drain() {
      while (pending?.length) await handle(pending.shift());
      if (pending) pending = null;
    },
  };
}
