/**
 * Inline card rendered when the desktop detects a version-1 bare-JSON
 * permission-request sentinel in a kind:9 message body. Mirrors the
 * `ConfigNudgeCard` pattern.
 *
 * Wire format: the harness signs a bare JSON object `{"v":1,"state":"pending",…}`
 * as the kind:9 event content. No code-fence wrapper — non-JSON content and
 * JSON without `"v":1` render as ordinary markdown.
 *
 * Security invariants enforced by the caller (`MessageRow`):
 * - `request` is only non-null when the kind-9 signer equals the known agent
 *   pubkey for this channel (D1 signer gate in `computePermissionRequest`).
 * - Resolved state (`state === "resolved"`) requires the edit to have been
 *   signed by the original agent (edit authenticity gate).
 * - Only an agent-signed kind-40003 edit may overlay the sentinel body —
 *   enforced in `formatTimelineMessages` before `computePermissionRequest` runs.
 *
 * Actionable buttons render ONLY when:
 *   (a) `request.state === "pending"` AND
 *   (b) `isOwner` is true (the current viewer is the verified agent owner).
 * All other viewers see a read-only card.
 */
import * as React from "react";
import { ShieldCheck } from "lucide-react";

import { startPermissionDecisionDelivery } from "@/features/agents/lib/permissionDecisionDelivery";
import { cn } from "@/shared/lib/cn";
import {
  Attachment,
  AttachmentContent,
  AttachmentMedia,
  AttachmentTitle,
} from "@/shared/ui/attachment";
import type {
  PermissionRequestPayload,
  PermissionRequestPending,
} from "@/shared/lib/permissionRequest";

export type PermissionRequestCardProps = {
  className?: string;
  request: PermissionRequestPayload;
  /** Hex pubkey of the agent that published the sentinel. */
  agentPubkey: string;
  /** Channel ID for routing the permission decision. */
  channelId: string;
  /**
   * True when the current viewer is the verified agent owner.
   * Absent or false → read-only card (buttons suppressed).
   */
  isOwner?: boolean;
  /**
   * Delivery function injected by tests so component tests can control
   * the outcome without a real relay. Production callers omit this and
   * get `startPermissionDecisionDelivery` by default.
   *
   * @internal — test seam only; not part of the public API.
   */
  _deliveryFn?: typeof startPermissionDecisionDelivery;
};

/**
 * Heuristic: treat an option as "deny" when its harness label contains deny,
 * reject, or block (case-insensitive). Opaque optionIds carry no inherent
 * semantics — the label is the only display hint available.
 */
function isDenyLabel(label: string): boolean {
  const lower = label.toLowerCase();
  return (
    lower.includes("deny") ||
    lower.includes("reject") ||
    lower.includes("block")
  );
}

function buttonClass(deny: boolean): string {
  return deny
    ? "rounded px-2 py-0.5 text-xs font-medium border border-destructive/40 text-destructive hover:bg-destructive/10 disabled:opacity-50"
    : "rounded px-2 py-0.5 text-xs font-medium border border-green-600/40 text-green-700 dark:text-green-400 hover:bg-green-600/10 disabled:opacity-50";
}

/**
 * Outcome display label — maps the harness outcome string to human copy.
 *
 * Verb derivation uses the positional allow contract: `optionIds[0]` is always
 * the `allow_once` action and `optionIds[1]` is always the `reject_once` action
 * (fixed order guaranteed by `sentinel_option_fields` in `crates/buzz-acp/src/acp.rs`
 * and pinned by the cross-language fixture `crates/buzz-acp/tests/fixtures/sentinel_pending.json`).
 * Deriving the verb from this signed identity — not the mutable display label — ensures
 * that a reject option named "Allow file access" still renders as "Denied", not "Approved".
 */
function outcomeLabel(
  outcome: string,
  chosenOptionId: string | null,
  labels: Record<string, string>,
  allowOptionId: string,
): string {
  if (outcome === "applied" && chosenOptionId !== null) {
    const chosen = labels[chosenOptionId];
    if (chosenOptionId === allowOptionId) {
      return chosen ? `Approved: ${chosen}` : "Approved";
    }
    // chosenOptionId is the reject option
    return chosen ? `Denied: ${chosen}` : "Denied";
  }
  if (outcome === "timed_out") return "Timed out";
  if (outcome === "cancelled") return "Cancelled";
  if (outcome === "rejected") return "Denied";
  return outcome;
}

/**
 * Allow/Deny buttons for a pending, owner-visible permission card.
 * On click: disables locally and shows "Decision sent". Convergence to final
 * state comes from the agent's kind-40003 edit or expiry — no promise of
 * immediate resolution from the harness response.
 */
function PermissionButtons({
  agentPubkey,
  channelId,
  request,
  nowSecs,
  deliveryFn = startPermissionDecisionDelivery,
}: {
  agentPubkey: string;
  channelId: string;
  request: PermissionRequestPending;
  /** Current time in seconds (driven by a parent ticking state). */
  nowSecs: number;
  /**
   * Delivery function — defaults to `startPermissionDecisionDelivery`.
   * Injected by tests to control the outcome without a real relay.
   */
  deliveryFn?: typeof startPermissionDecisionDelivery;
}) {
  const [submitted, setSubmitted] = React.useState<string | null>(null);

  const expired = request.expiresAt <= nowSecs;

  if (expired) {
    return (
      <div
        aria-live="polite"
        className="mt-1.5 text-xs text-muted-foreground"
        role="status"
      >
        Timed out
      </div>
    );
  }

  if (submitted !== null) {
    return (
      <div
        aria-live="polite"
        className="mt-1.5 text-xs text-muted-foreground"
        role="status"
      >
        Decision sent
      </div>
    );
  }

  return (
    <div className="mt-1.5 flex flex-col gap-2">
      <div className="flex flex-wrap gap-1.5">
        {request.optionIds.map((optionId) => {
          const label = request.labels[optionId] ?? optionId;
          return (
            <button
              key={optionId}
              type="button"
              className={buttonClass(isDenyLabel(label))}
              data-testid={`permission-decision-${optionId}`}
              onClick={() => {
                // Recheck expiry at click time — prevents submitting a
                // decision on a card that expired between renders.
                if (request.expiresAt <= Date.now() / 1000) return;
                setSubmitted(optionId);
                void deliveryFn({
                  agentPubkey,
                  channelId,
                  requestNonce: request.requestNonce,
                  optionId,
                  deadlineSecs: request.expiresAt,
                })
                  .then((outcome) => {
                    // `"failed"` means the harness received the frame but could
                    // not route it (no_active_turn / channel_closed / no_channel)
                    // — re-enable so the owner can retry. `"acked"` and
                    // `"expired"` are terminal: the harness applied the decision
                    // or the card timed out; the card transitions away via the
                    // kind-40003 edit or expiry countdown and no retry is needed.
                    // `"channel_full"` is transient: the retransmit loop stays
                    // active and keeps resending — no re-enable needed here.
                    if (outcome === "failed") setSubmitted(null);
                  })
                  .catch(() => {
                    // The delivery loop never rejects — it resolves one of
                    // "acked" | "expired" | "failed". This branch guards against
                    // any unexpected error and re-enables for safety.
                    setSubmitted(null);
                  });
              }}
            >
              {label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/**
 * Countdown display for a pending card. Updates the shared `now` state
 * every second until expiry so that both the countdown and the button
 * actionability are driven by the same tick.
 */
function ExpiryCountdown({
  expiresAt,
  nowSecs,
}: {
  expiresAt: number;
  nowSecs: number;
}) {
  const secsLeft = Math.max(0, Math.round(expiresAt - nowSecs));

  if (secsLeft <= 0) return null;
  const mins = Math.floor(secsLeft / 60);
  const secs = secsLeft % 60;
  const label = mins > 0 ? `${mins}m ${secs}s` : `${secs}s`;
  return (
    <span className="text-2xs text-muted-foreground opacity-70">
      {" "}
      · expires in {label}
    </span>
  );
}

export function PermissionRequestCard({
  className,
  request,
  agentPubkey,
  channelId,
  isOwner,
  _deliveryFn,
}: PermissionRequestCardProps) {
  if (request.state === "resolved") {
    const resolvedLabel = outcomeLabel(
      request.outcome,
      request.chosenOptionId,
      request.labels,
      request.optionIds[0] ?? "",
    );
    return (
      <Attachment
        className={cn(
          "max-w-[min(100%,32rem)] shrink-0 shadow-none",
          className,
        )}
        orientation="horizontal"
        state="done"
      >
        <AttachmentMedia className="text-muted-foreground">
          <ShieldCheck aria-hidden="true" className="h-4 w-4" />
        </AttachmentMedia>
        <AttachmentContent>
          <AttachmentTitle className="whitespace-normal text-muted-foreground line-clamp-2">
            Permission request resolved
          </AttachmentTitle>
          {request.description ? (
            <div className="mt-0.5 text-xs text-muted-foreground line-clamp-2">
              {request.description}
            </div>
          ) : null}
          <div className="mt-0.5 text-xs text-muted-foreground">
            {resolvedLabel}
          </div>
        </AttachmentContent>
      </Attachment>
    );
  }

  // Pending state — one ticking `nowSecs` drives both the countdown display
  // and button actionability so expiry is observed atomically.
  return (
    <PendingPermissionRequestCard
      agentPubkey={agentPubkey}
      channelId={channelId}
      className={className}
      isOwner={isOwner}
      request={request}
      deliveryFn={_deliveryFn}
    />
  );
}

/**
 * Pending-state card. Owns the ticking `nowSecs` state so that
 * `ExpiryCountdown` and `PermissionButtons` always see the same clock value.
 */
function PendingPermissionRequestCard({
  className,
  request,
  agentPubkey,
  channelId,
  isOwner,
  deliveryFn,
}: {
  className?: string;
  request: PermissionRequestPending;
  agentPubkey: string;
  channelId: string;
  isOwner?: boolean;
  deliveryFn?: typeof startPermissionDecisionDelivery;
}) {
  const [nowSecs, setNowSecs] = React.useState(() => Date.now() / 1000);

  React.useEffect(() => {
    const id = setInterval(() => {
      const now = Date.now() / 1000;
      setNowSecs(now);
      if (now >= request.expiresAt) clearInterval(id);
    }, 1000);
    // In Node test environments (not browsers), intervals can keep the process
    // alive. Call unref() when available to allow clean test exits.
    (id as unknown as { unref?: () => void }).unref?.();
    return () => clearInterval(id);
  }, [request.expiresAt]);

  const expired = request.expiresAt <= nowSecs;

  return (
    <Attachment
      className={cn("max-w-[min(100%,32rem)] shrink-0 shadow-none", className)}
      orientation="horizontal"
      state={expired ? "done" : "idle"}
    >
      <AttachmentMedia className="text-amber-600 dark:text-amber-400">
        <ShieldCheck aria-hidden="true" className="h-4 w-4" />
      </AttachmentMedia>
      <AttachmentContent>
        <AttachmentTitle className="whitespace-normal text-amber-700 dark:text-amber-400 line-clamp-2">
          Permission request
          {!expired ? (
            <ExpiryCountdown expiresAt={request.expiresAt} nowSecs={nowSecs} />
          ) : null}
        </AttachmentTitle>
        {request.description ? (
          <div className="mt-0.5 text-xs text-muted-foreground line-clamp-3">
            {request.description}
          </div>
        ) : null}
        {isOwner ? (
          <PermissionButtons
            agentPubkey={agentPubkey}
            channelId={channelId}
            nowSecs={nowSecs}
            request={request}
            deliveryFn={deliveryFn}
          />
        ) : (
          <div className="mt-1 text-xs text-muted-foreground">
            Waiting for owner approval
          </div>
        )}
      </AttachmentContent>
    </Attachment>
  );
}
