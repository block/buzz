import * as React from "react";

import {
  createArtilleryFinishedEvent,
  createArtilleryTurnResolvedEvent,
  parseArtilleryDurableEvent,
  recoverArtilleryMatch,
  type ArtilleryDurableEvent,
} from "@/features/games/artillery/durableProtocol";
import { formatArtilleryLifecycleMessage } from "@/features/games/artillery/channelEvent";
import { createManagedArtilleryAgent } from "@/features/games/artillery/liveAgentAdapter";
import { liveArtilleryMatchController } from "@/features/games/artillery/liveMatchController";
import {
  cacheDurableMatchEvent,
  readDurableMatchCache,
} from "@/features/games/artillery/durableMatchCache";
import { artilleryRefereeHostSession } from "@/features/games/artillery/refereeHostSession";
import {
  artilleryRefereeLeaseMs,
  parseArtilleryRefereeLeaseEvent,
  recoverArtilleryRefereeLease,
  type ArtilleryRefereeLeaseEvent,
} from "@/features/games/artillery/refereeLease";
import { relayClient } from "@/shared/api/relayClient";
import {
  getEventById,
  getThreadReplies,
  sendChannelMessage,
} from "@/shared/api/tauri";
import type { RelayEvent, ThreadCursor } from "@/shared/api/types";

type DurableEventRecord = {
  createdAt: number;
  event: ArtilleryDurableEvent;
  eventId: string;
};

type LeaseEventRecord = {
  event: ArtilleryRefereeLeaseEvent;
  eventId: string;
};

/**
 * Hydrates and follows a match thread so reloads and spectator clients render
 * the same deterministic arena state as the referee host.
 */
export function DurableMatchHydrator({
  channelId,
  matchId,
  rootEventId,
}: {
  channelId: string;
  matchId: string;
  rootEventId: string;
}) {
  const [status, setStatus] = React.useState<
    "loading" | "watching" | "taking-over" | "hosting" | "complete" | "error"
  >("loading");
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    let unsubscribe: (() => Promise<void>) | undefined;
    const records = new Map<string, DurableEventRecord>();
    const leaseRecords = new Map<string, LeaseEventRecord>();
    let latestRecovered: ReturnType<typeof recoverArtilleryMatch> = null;
    let rootCreatedAt = Date.now();
    let takeoverInFlight = false;

    const sortedDurableEvents = () =>
      [...records.values()]
        .sort(
          (left, right) =>
            left.createdAt - right.createdAt ||
            left.eventId.localeCompare(right.eventId),
        )
        .map((record) => record.event);

    const publishLifecycle = (content: string) =>
      sendChannelMessage(channelId, content, rootEventId);

    const applyEvent = (relayEvent: RelayEvent) => {
      const event = parseArtilleryDurableEvent(relayEvent.content);
      const lease = parseArtilleryRefereeLeaseEvent(relayEvent.content);
      if (event?.matchId === matchId) {
        records.set(relayEvent.id, {
          createdAt: relayEvent.created_at,
          event,
          eventId: relayEvent.id,
        });
      }
      if (lease?.matchId === matchId) {
        leaseRecords.set(relayEvent.id, {
          event: lease,
          eventId: relayEvent.id,
        });
      }
      if (!event && !lease) return;
      cacheDurableMatchEvent(channelId, rootEventId, relayEvent);
      const recovered = recoverArtilleryMatch(sortedDurableEvents(), matchId);
      if (!recovered) return;
      latestRecovered = recovered;
      liveArtilleryMatchController.hydrate({
        channelId,
        match: recovered.match,
        matchComplete: recovered.complete,
        statusEventId: rootEventId,
        timeoutMs: recovered.timeoutMs,
      });
      setStatus(recovered.complete ? "complete" : "watching");
    };

    const attemptTakeover = async () => {
      const recovered = latestRecovered;
      if (
        cancelled ||
        takeoverInFlight ||
        !recovered ||
        recovered.complete ||
        artilleryRefereeHostSession.getActive()?.matchId === matchId
      ) {
        return;
      }
      const now = Date.now();
      const leases = [...leaseRecords.values()].map((record) => record.event);
      const currentLease = recoverArtilleryRefereeLease(leases, matchId, now);
      if (currentLease?.active) return;
      if (!currentLease && now < rootCreatedAt + artilleryRefereeLeaseMs()) {
        return;
      }

      takeoverInFlight = true;
      setStatus("taking-over");
      const ownerId = crypto.randomUUID();
      const term = (currentLease?.term ?? 0) + 1;
      try {
        const claimed = await artilleryRefereeHostSession.start({
          channelId,
          leaseMs: artilleryRefereeLeaseMs(),
          matchId,
          onLeaseLost: () => liveArtilleryMatchController.yieldReferee(),
          ownerId,
          rootEventId,
          term,
        });
        leaseRecords.set(claimed.result.eventId, {
          event: claimed.event,
          eventId: claimed.result.eventId,
        });
        await new Promise((resolve) => window.setTimeout(resolve, 750));
        const elected = recoverArtilleryRefereeLease(
          [...leaseRecords.values()].map((record) => record.event),
          matchId,
        );
        if (
          !elected?.active ||
          elected.ownerId !== ownerId ||
          elected.term !== term
        ) {
          await artilleryRefereeHostSession.stop(false);
          setStatus("watching");
          return;
        }

        setStatus("hosting");
        const red = createManagedArtilleryAgent({
          agent: {
            name: recovered.match.agents.red.name,
            pubkey: recovered.match.agents.red.id,
          },
          channelId,
          responseTimeoutMs: recovered.timeoutMs,
          side: "red",
          threadRootEventId: rootEventId,
        });
        const blue = createManagedArtilleryAgent({
          agent: {
            name: recovered.match.agents.blue.name,
            pubkey: recovered.match.agents.blue.id,
          },
          channelId,
          responseTimeoutMs: recovered.timeoutMs,
          side: "blue",
          threadRootEventId: rootEventId,
        });
        void liveArtilleryMatchController
          .start({
            agents: { blue, red },
            channelId,
            id: matchId,
            maxTurns: recovered.maxTurns,
            onMatchComplete: async (match) => {
              await publishLifecycle(
                formatArtilleryLifecycleMessage(
                  createArtilleryFinishedEvent(match),
                ),
              );
            },
            onTurnResolved: async ({ state, turn }) => {
              await publishLifecycle(
                formatArtilleryLifecycleMessage(
                  createArtilleryTurnResolvedEvent(state, turn),
                ),
              );
            },
            resumeMatch: recovered.match,
            statusEventId: rootEventId,
            timeoutMs: recovered.timeoutMs,
          })
          .catch(() => {})
          .finally(() => {
            void artilleryRefereeHostSession.stop();
          });
      } catch (cause) {
        setStatus("error");
        setError(
          cause instanceof Error ? cause.message : "Referee takeover failed",
        );
      } finally {
        takeoverInFlight = false;
      }
    };

    const load = async () => {
      try {
        for (const event of readDurableMatchCache(channelId, rootEventId)) {
          applyEvent(event);
        }
        unsubscribe = await relayClient.subscribeToChannelLive(
          channelId,
          applyEvent,
        );
        const root = await getEventById(rootEventId);
        rootCreatedAt =
          root.created_at > 10_000_000_000
            ? root.created_at
            : root.created_at * 1_000;
        applyEvent(root);
        let cursor: ThreadCursor | null = null;
        do {
          const page = await getThreadReplies(rootEventId, channelId, {
            cursor,
            limit: 500,
          });
          for (const event of page.events) applyEvent(event);
          cursor = page.nextCursor;
        } while (cursor && !cancelled);
        if (!cancelled && records.size === 0) {
          throw new Error("No durable match events were found in this thread.");
        }
      } catch (cause) {
        if (cancelled) return;
        if (records.size > 0) return;
        setStatus("error");
        setError(
          cause instanceof Error ? cause.message : "Could not recover match",
        );
      }
    };

    void load();
    const takeoverTimer = window.setInterval(() => {
      void attemptTakeover();
    }, 500);
    return () => {
      cancelled = true;
      window.clearInterval(takeoverTimer);
      if (unsubscribe) void unsubscribe().catch(() => {});
    };
  }, [channelId, matchId, rootEventId]);

  return (
    <div
      className="rounded-xl border border-sky-500/20 bg-sky-500/5 px-3 py-2 text-xs text-muted-foreground"
      data-testid="durable-match-status"
      data-watch-status={status}
    >
      {status === "loading"
        ? "Loading canonical match history…"
        : status === "watching"
          ? "Watching the active channel referee. Automatic takeover is armed."
          : status === "taking-over"
            ? "The referee lease expired. Electing this client as replacement…"
            : status === "hosting"
              ? "This client took over the referee and resumed the match."
              : status === "complete"
                ? "Recovered the complete match from its channel thread."
                : `Recovery failed: ${error}`}
    </div>
  );
}
