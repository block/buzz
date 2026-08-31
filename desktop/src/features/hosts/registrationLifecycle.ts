import type { HostPublicationJournal } from "./pendingPublication";
import {
  reconcileHost,
  type HostBridge,
  type HostRelay,
  type HostSnapshot,
} from "./registration";

/** One effect's cancellable publisher. Refresh bursts coalesce, never overlap. */
export function createHostRegistrationLifecycle(args: {
  owner: string;
  bridge: HostBridge;
  journal: HostPublicationJournal;
  connect: () => HostRelay & { disconnect(): void };
  now: () => number;
  checking: () => void;
  success: (snapshot: HostSnapshot) => void;
  failure: (error: unknown) => void;
  after?: Promise<void>;
}) {
  let active = true;
  let pending = false;
  let running: Promise<void> | undefined;
  let client: ReturnType<typeof args.connect> | undefined;
  const drain = async () => {
    await args.after;
    while (active && pending) {
      pending = false;
      args.checking();
      try {
        client = args.connect();
        const snapshot = await reconcileHost({
          owner: args.owner,
          relay: client,
          bridge: args.bridge,
          journal: args.journal,
          active: () => active,
          now: args.now,
        });
        if (active) args.success(snapshot);
      } catch (error) {
        if (active) args.failure(error);
      } finally {
        client?.disconnect();
        client = undefined;
      }
    }
  };
  const refresh = (): Promise<void> => {
    if (!active) return running ?? Promise.resolve();
    pending = true;
    if (!running)
      running = drain().finally(() => {
        running = undefined;
        // A refresh may arrive after drain returns but before this microtask.
        if (active && pending) return refresh();
      });
    return running;
  };
  return {
    refresh,
    stop(): Promise<void> {
      active = false;
      pending = false;
      client?.disconnect();
      return running ?? Promise.resolve();
    },
  };
}
