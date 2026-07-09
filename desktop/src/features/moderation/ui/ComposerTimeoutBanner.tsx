import { Clock } from "lucide-react";

import { formatTimeoutRemaining } from "@/features/moderation/lib/timeout";

/**
 * A banner docked to the top edge of the composer while the member is timed
 * out by community moderators. Shows a live countdown when the expiry is known;
 * otherwise states the block without a timer (the relay gave no timestamp).
 *
 * Purely presentational — the timeout state and its per-second tick are owned
 * by {@link useTimeoutState}; this only formats what it is handed.
 */
export function ComposerTimeoutBanner({
  expiresAtMs,
}: {
  /** Timeout expiry in epoch ms, or null when the relay gave no timestamp. */
  expiresAtMs: number | null;
}) {
  const remaining = formatTimeoutRemaining(expiresAtMs);

  return (
    <div
      className="relative z-0 mx-5 -mb-3 flex items-center gap-2 rounded-t-2xl border border-b-0 border-amber-500/30 bg-amber-500/15 px-4 pb-5 pt-2.5 text-sm leading-5 text-foreground backdrop-blur-sm"
      data-testid="composer-timeout-banner"
    >
      <Clock aria-hidden className="h-4 w-4 shrink-0 text-amber-600" />
      <span className="min-w-0">
        {remaining
          ? `You're timed out by community moderators — ${remaining} left.`
          : "You're timed out by community moderators."}
      </span>
    </div>
  );
}
