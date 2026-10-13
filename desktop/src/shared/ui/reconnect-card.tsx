// Calm, theme-matching connection status card: a spinner while reconnecting, a brief "Back online" that fades on its own,
// and an optional (never required) "Reconnect now" once an outage has lasted a while. Nothing here needs a click or a
// dismiss; the owner just passes the live state.
import * as React from "react";
import { Check } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";

export const RECONNECT_NOW_AFTER_MS = 20_000;
const BACK_ONLINE_MS = 2_200;

type Props = {
  /** true while the connection is down and being retried */
  reconnecting: boolean;
  title: string;
  description?: string;
  /** When this outage started (ms); enables "Reconnect now" after RECONNECT_NOW_AFTER_MS. */
  failingSince?: number;
  onReconnectNow?: () => void;
  backOnlineLabel?: string;
  className?: string;
  testId?: string;
};

/** Shows the card while reconnecting and for a moment after recovery, then unmounts itself. */
export function ReconnectCard({
  reconnecting,
  title,
  description,
  failingSince,
  onReconnectNow,
  backOnlineLabel = "Back online",
  className,
  testId = "reconnect-card",
}: Props) {
  const [showBack, setShowBack] = React.useState(false);
  const [fading, setFading] = React.useState(false);
  const was = React.useRef(reconnecting);
  const [, tick] = React.useReducer((x: number) => x + 1, 0);

  React.useEffect(() => {
    if (was.current && !reconnecting) {
      setShowBack(true);
      setFading(false);
      const f = window.setTimeout(() => setFading(true), BACK_ONLINE_MS - 500);
      const t = window.setTimeout(() => setShowBack(false), BACK_ONLINE_MS);
      was.current = reconnecting;
      return () => {
        window.clearTimeout(f);
        window.clearTimeout(t);
      };
    }
    if (reconnecting) setShowBack(false);
    was.current = reconnecting;
  }, [reconnecting]);

  // re-render once the "Reconnect now" threshold passes
  React.useEffect(() => {
    if (!reconnecting || !failingSince) return;
    const left = failingSince + RECONNECT_NOW_AFTER_MS - Date.now();
    if (left <= 0) return;
    const t = window.setTimeout(tick, left + 50);
    return () => window.clearTimeout(t);
  }, [reconnecting, failingSince]);

  if (!reconnecting && !showBack) return null;
  const offerNow =
    reconnecting &&
    !!onReconnectNow &&
    !!failingSince &&
    Date.now() - failingSince >= RECONNECT_NOW_AFTER_MS;

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid={testId}
      data-state={reconnecting ? "reconnecting" : "online"}
      className={cn(
        "buzz-reconnect-card flex items-center gap-3 rounded-2xl border border-border/60 bg-card/95 px-3.5 py-2.5 text-sm text-foreground shadow-sm backdrop-blur transition-opacity duration-500",
        fading ? "opacity-0" : "opacity-100",
        className,
      )}
    >
      <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        {reconnecting ? (
          <Spinner aria-hidden="true" className="h-4 w-4 border-2" />
        ) : (
          <Check
            aria-hidden="true"
            className="h-4 w-4 text-emerald-600 dark:text-emerald-400"
          />
        )}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">
          {reconnecting ? title : backOnlineLabel}
        </span>
        {reconnecting && description ? (
          <span className="block truncate text-xs text-muted-foreground">
            {description}
          </span>
        ) : null}
      </span>
      {offerNow ? (
        <button
          type="button"
          data-testid={`${testId}-now`}
          onClick={onReconnectNow}
          className="shrink-0 rounded-full border border-border/70 px-2.5 py-1 text-xs text-muted-foreground hover:bg-muted"
        >
          Reconnect now
        </button>
      ) : null}
    </div>
  );
}
