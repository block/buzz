import { listen } from "@tauri-apps/api/event";
import { Headphones } from "lucide-react";
import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { useHuddle } from "../HuddleContext";
import { useHeadphonesGate } from "../lib/useHeadphonesGate";
import { HeadphonesGate } from "./HeadphonesGate";

/** Huddle lifecycle event kinds */
const KIND_HUDDLE_STARTED = 48100;
const KIND_HUDDLE_PARTICIPANT_JOINED = 48101;
const KIND_HUDDLE_PARTICIPANT_LEFT = 48102;
const KIND_HUDDLE_ENDED = 48103;

type ActiveHuddle = {
  ephemeralChannelId: string;
  participants: Set<string>;
};

type HuddleIndicatorProps = {
  channelId: string;
  className?: string;
  /** Called when the user clicks the button and no huddle is active (start). */
  onStart?: () => void;
  /** Whether the start action is disabled (e.g., permissions, already starting). */
  startDisabled?: boolean;
};

/**
 * Detects active huddles in a channel via kind:48100-48103 events.
 * Shows a glowing headphone icon when a huddle is active, with participant count.
 * Click to join the huddle.
 */
export function HuddleIndicator({
  channelId,
  className,
  onStart,
  startDisabled,
}: HuddleIndicatorProps) {
  const { joinHuddle, isStarting } = useHuddle();
  const queryClient = useQueryClient();
  const [activeHuddle, setActiveHuddle] = React.useState<ActiveHuddle | null>(
    null,
  );
  const [isJoining, setIsJoining] = React.useState(false);

  // Pre-join "use headphones" confirmation while echo cancellation is
  // missing. Mirrors the `aecMissing` constant in HuddleBar — both flip
  // to false in the AEC follow-up PR, after which this hook is a no-op
  // and the gate UI/components are mechanically deletable.
  const aecMissing = true;
  const headphonesGate = useHeadphonesGate(aecMissing);

  React.useEffect(() => {
    if (!channelId) return;

    let disposed = false;
    let cleanup: (() => void) | null = null;

    // Track all seen events for reconstruction. Keyed by event.id for dedup.
    const seenEvents = new Map<string, RelayEvent>();

    /** Reconstruct huddle state from the full set of seen events.
     *  Sort by created_at, then kind (causal: start < join < left < end),
     *  then event id for final tiebreak. This handles out-of-order delivery,
     *  reconnect replay, late mounts, and same-second event batches.
     *
     *  Resilient to missing start event: if we see join/left events for an
     *  ephemeral channel without a prior start, we infer the huddle exists.
     *  This covers the edge case where >100 lifecycle events push the start
     *  event out of the subscription window. */
    function reconstruct() {
      const sorted = [...seenEvents.values()].sort(
        (a, b) =>
          a.created_at - b.created_at ||
          a.kind - b.kind ||
          a.id.localeCompare(b.id),
      );

      let huddle: ActiveHuddle | null = null;
      // Track ended ephemeral channels so late-arriving join/left events
      // (e.g. relay-emitted 48102 that lands 1s after a client-emitted 48103)
      // don't resurrect a phantom huddle via the "infer huddle exists" fallback.
      const endedChannels = new Set<string>();

      for (const ev of sorted) {
        let ephId: string | null = null;
        try {
          const content = JSON.parse(ev.content);
          ephId = content.ephemeral_channel_id ?? null;
        } catch {
          continue; // Malformed — skip
        }

        switch (ev.kind) {
          case KIND_HUDDLE_STARTED: {
            if (!ephId) break;
            // A new start supersedes any previous ended state for this channel.
            endedChannels.delete(ephId);
            huddle = {
              ephemeralChannelId: ephId,
              participants: new Set([ev.pubkey]),
            };
            break;
          }
          case KIND_HUDDLE_PARTICIPANT_JOINED: {
            if (!ephId) break;
            // Skip if this ephemeral channel has already ended — don't
            // resurrect a phantom huddle from a late-arriving relay event.
            if (endedChannels.has(ephId)) break;
            // 48101 events are relay-signed — the actual participant is in the "p" tag.
            const joinedPk =
              ev.tags.find((t) => t[0] === "p")?.[1] ?? ev.pubkey;
            if (!huddle || ephId !== huddle.ephemeralChannelId) {
              huddle = {
                ephemeralChannelId: ephId,
                participants: new Set(),
              };
            }
            huddle.participants.add(joinedPk);
            break;
          }
          case KIND_HUDDLE_PARTICIPANT_LEFT: {
            if (!ephId) break;
            // Skip if this ephemeral channel has already ended.
            if (endedChannels.has(ephId)) break;
            // 48102 events are relay-signed — the actual participant is in the "p" tag.
            const leftPk = ev.tags.find((t) => t[0] === "p")?.[1] ?? ev.pubkey;
            if (!huddle || ephId !== huddle.ephemeralChannelId) {
              huddle = {
                ephemeralChannelId: ephId,
                participants: new Set(),
              };
            }
            huddle.participants.delete(leftPk);
            break;
          }
          case KIND_HUDDLE_ENDED: {
            if (!ephId) break;
            endedChannels.add(ephId);
            if (huddle && ephId === huddle.ephemeralChannelId) {
              huddle = null;
            }
            break;
          }
        }
      }

      if (!disposed) {
        setActiveHuddle(huddle);
      }
    }

    // Subscribe to huddle lifecycle events only (kinds 48100–48103).
    // limit: 100 covers long-lived huddles with many join/leave cycles.
    relayClient
      .subscribeToHuddleEvents(channelId, (event: RelayEvent) => {
        if (disposed) return;

        // Dedup by event ID — ignore replayed events from reconnect.
        if (seenEvents.has(event.id)) return;
        seenEvents.set(event.id, event);

        // Reconstruct from full history on every new event.
        // This is cheap — huddle lifecycle events are rare (typically <20).
        reconstruct();
      })
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((err) => {
        console.error("[HuddleIndicator] subscription failed:", err);
      });

    return () => {
      disposed = true;
      cleanup?.();
      setActiveHuddle(null);
    };
  }, [channelId]);

  // When the local user ends/leaves a huddle, the backend transitions to idle
  // and emits huddle-state-changed. Clear the indicator immediately rather than
  // waiting for the relay's 48103 event (which may arrive late or not at all
  // if the relay connection tears down first).
  React.useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    listen<{ phase: string }>("huddle-state-changed", (event) => {
      if (!cancelled && event.payload.phase === "idle") {
        setActiveHuddle(null);
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Pre-join headphones confirmation. The same dialog instance serves
  // both the start and the join paths — `gate` decides which deferred
  // action to fire on Continue.
  const gateDialog = (
    <HeadphonesGate
      open={headphonesGate.dialogOpen}
      onContinue={headphonesGate.onContinue}
      onCancel={headphonesGate.onCancel}
    />
  );

  // No active huddle — render the start button (if onStart provided).
  if (!activeHuddle) {
    if (!onStart) return null;
    return (
      <>
        <Button
          aria-label="Start huddle"
          className={cn("h-7 w-7 rounded-full", className)}
          data-testid="channel-start-huddle-trigger"
          disabled={startDisabled || isStarting}
          onClick={() => headphonesGate.gate(() => onStart())}
          size="icon"
          type="button"
          variant="outline"
        >
          <Headphones className="h-3 w-3" />
        </Button>
        {gateDialog}
      </>
    );
  }

  // At least 1 participant must exist for the huddle to be active.
  // When START fell out of the event window, the creator isn't in the
  // reconstructed set — floor at 1 to avoid showing "0 participants".
  const participantCount = Math.max(1, activeHuddle.participants.size);

  async function doJoin() {
    if (!activeHuddle || isJoining) return;
    setIsJoining(true);
    try {
      await joinHuddle(channelId, activeHuddle.ephemeralChannelId);
      // Refetch channels so the ephemeral channel appears in the sidebar.
      void queryClient.invalidateQueries({ queryKey: ["channels"] });
    } catch (e) {
      console.error("Failed to join huddle:", e);
    } finally {
      setIsJoining(false);
    }
  }

  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            aria-label={`Join active huddle (${participantCount} participant${participantCount !== 1 ? "s" : ""})`}
            className={cn("relative h-7 w-7 rounded-full", className)}
            disabled={isJoining || isStarting}
            onClick={() => headphonesGate.gate(() => void doJoin())}
            size="icon"
            type="button"
            variant="outline"
          >
            <Headphones className="h-3 w-3 text-muted-foreground" />
            <span className="absolute inset-0 animate-pulse rounded-full ring-2 ring-border/70" />
            {/* Participant count badge */}
            {participantCount > 0 && (
              <span className="absolute -right-1 -top-1 flex h-3.5 min-w-3.5 items-center justify-center rounded-full border border-border bg-background px-0.5 text-[9px] font-bold text-muted-foreground">
                {participantCount}
              </span>
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent>
          {`Huddle active — ${participantCount} participant${participantCount !== 1 ? "s" : ""}`}
        </TooltipContent>
      </Tooltip>
      {gateDialog}
    </>
  );
}
