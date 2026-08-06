import {
  createArtilleryRefereeLeaseEvent,
  formatArtilleryRefereeLeaseMessage,
  parseArtilleryRefereeLeaseEvent,
} from "@/features/games/artillery/refereeLease";
import { relayClient } from "@/shared/api/relayClient";
import { sendChannelMessage } from "@/shared/api/tauri";

type HostSession = {
  matchId: string;
  ownerId: string;
  stop: (release?: boolean) => Promise<void>;
  term: number;
};

let activeSession: HostSession | null = null;

async function publishLease({
  action,
  channelId,
  leaseMs,
  matchId,
  ownerId,
  rootEventId,
  term,
}: {
  action: "claim" | "renew" | "release";
  channelId: string;
  leaseMs: number;
  matchId: string;
  ownerId: string;
  rootEventId: string;
  term: number;
}) {
  const event = createArtilleryRefereeLeaseEvent({
    action,
    leaseMs,
    matchId,
    ownerId,
    term,
  });
  const result = await sendChannelMessage(
    channelId,
    formatArtilleryRefereeLeaseMessage(event),
    rootEventId,
  );
  return { event, result };
}

export const artilleryRefereeHostSession = {
  getActive() {
    return activeSession;
  },

  async start(input: {
    channelId: string;
    leaseMs: number;
    matchId: string;
    ownerId: string;
    onLeaseLost?: () => void;
    rootEventId: string;
    term: number;
  }) {
    await activeSession?.stop(false);
    const claimed = await publishLease({ ...input, action: "claim" });
    let stopped = false;
    let renewing = false;
    let unsubscribe: (() => Promise<void>) | undefined;
    const timer = window.setInterval(
      () => {
        if (stopped || renewing) return;
        renewing = true;
        void publishLease({ ...input, action: "renew" })
          .catch(() => {})
          .finally(() => {
            renewing = false;
          });
      },
      Math.max(500, Math.floor(input.leaseMs / 3)),
    );
    const session: HostSession = {
      matchId: input.matchId,
      ownerId: input.ownerId,
      term: input.term,
      stop: async (release = true) => {
        if (stopped) return;
        stopped = true;
        window.clearInterval(timer);
        if (unsubscribe) await unsubscribe().catch(() => {});
        if (activeSession === session) activeSession = null;
        if (release) {
          await publishLease({ ...input, action: "release" }).catch(() => {});
        }
      },
    };
    activeSession = session;
    try {
      unsubscribe = await relayClient.subscribeToChannelLive(
        input.channelId,
        (relayEvent) => {
          const lease = parseArtilleryRefereeLeaseEvent(relayEvent.content);
          if (!lease || lease.matchId !== input.matchId || stopped) return;
          const superseded =
            lease.term > input.term ||
            (lease.term === input.term &&
              lease.action === "claim" &&
              lease.ownerId.localeCompare(input.ownerId) < 0);
          if (superseded) {
            void session.stop(false).then(() => input.onLeaseLost?.());
          }
        },
      );
    } catch (cause) {
      await session.stop(false);
      throw cause;
    }
    return claimed;
  },

  async stop(release = true) {
    await activeSession?.stop(release);
  },
};
