export type TrayAction =
  | { kind: "newChannel" }
  | { kind: "openChannel"; channelId: string };

type TrayActionConsumerOptions = {
  addFocusListener: (listener: () => void) => () => void;
  listenForAvailable: (listener: () => void) => Promise<() => void>;
  takePendingActions: () => Promise<TrayAction[]>;
  requeueActions: (actions: TrayAction[]) => Promise<void>;
  handleAction: (action: TrayAction) => void;
};

/** Subscribes to native tray actions and drains anything queued before mount. */
export function subscribeToTrayActions({
  addFocusListener,
  listenForAvailable,
  takePendingActions,
  requeueActions,
  handleAction,
}: TrayActionConsumerOptions): () => void {
  let disposed = false;
  let drainAgain = false;
  let drainInFlight = false;
  let unlisten: (() => void) | undefined;

  const handlePendingActions = async () => {
    if (disposed) return;
    if (drainInFlight) {
      drainAgain = true;
      return;
    }

    drainInFlight = true;
    try {
      do {
        drainAgain = false;
        const actions = await takePendingActions();
        if (disposed) {
          if (actions.length > 0) {
            await requeueActions(actions);
          }
          return;
        }
        for (const action of actions) {
          handleAction(action);
        }
      } while (drainAgain);
    } finally {
      drainInFlight = false;
    }
  };

  const removeFocusListener = addFocusListener(() => {
    void handlePendingActions();
  });

  void (async () => {
    const nextUnlisten = await listenForAvailable(() => {
      void handlePendingActions();
    });
    if (disposed) {
      nextUnlisten();
      return;
    }
    unlisten = nextUnlisten;
    await handlePendingActions();
  })();

  return () => {
    disposed = true;
    removeFocusListener();
    unlisten?.();
  };
}
