import { CircleSlash, ScrollText, X } from "lucide-react";

import { Button } from "@/shared/ui/button";

const BANNER_CLASS =
  "relative z-0 -mb-4 flex transform-gpu items-center gap-2 rounded-t-2xl border border-b-0 border-border/60 bg-muted/55 px-4 pb-6 pt-2.5 text-sm leading-5 text-muted-foreground backdrop-blur-sm transition-colors";

/**
 * Discloses that this message will create a tracked NIP-AD obligation, and
 * lets the sender opt out.
 *
 * **Why this exists.** Mentioning an agent used to mark the message as a
 * request invisibly, so "@agent thanks for the help" — which asks for nothing
 * — became an obligation that showed as an unanswered gap until the agent
 * happened to speak again. And mentioning *two* agents silently dropped the
 * marker, turning a request the user clearly intended into untracked chat
 * with no signal that accountability had been switched off.
 *
 * Both were the same mistake: the composer decided something consequential
 * about the outbound event and told nobody. A request is a claim on someone
 * else's time that a third party can later audit, so the sender should be
 * able to see it before it goes out — and, in the common case, do nothing.
 */
export function ComposerRequestBanner({
  targetLabel,
  multiAgent,
  onDisableRequest,
}: {
  /** Display name of the single agent this request is addressed to. */
  targetLabel: string | null;
  /**
   * True when more than one agent is mentioned. v1 tracks exactly one
   * obligation per request, so nothing is tracked — said out loud rather
   * than left for the reader to notice.
   */
  multiAgent: boolean;
  /** Omitted when the sender has already opted out. */
  onDisableRequest?: () => void;
}) {
  if (multiAgent) {
    return (
      <div className={BANNER_CLASS} data-testid="composer-multi-agent-notice">
        <CircleSlash aria-hidden className="h-4 w-4 shrink-0" />
        <p className="min-w-0 flex-1">
          Mentions more than one agent, so this won't be tracked as a request.
          Address one agent to make it accountable.
        </p>
      </div>
    );
  }

  if (!targetLabel) {
    return null;
  }

  return (
    <div className={BANNER_CLASS} data-testid="composer-request-notice">
      <ScrollText aria-hidden className="h-4 w-4 shrink-0" />
      <p className="min-w-0 flex-1">
        Tracked as a request to{" "}
        <span className="font-medium text-foreground">{targetLabel}</span>
      </p>
      {onDisableRequest ? (
        <Button
          aria-label="Send without tracking this as a request"
          className="-mr-1 h-7 w-7 shrink-0 px-0 text-muted-foreground hover:text-foreground"
          onClick={onDisableRequest}
          size="icon"
          type="button"
          variant="ghost"
        >
          <X className="h-4 w-4" />
        </Button>
      ) : null}
    </div>
  );
}
