import { AlertCircle, CheckCircle2, ShieldCheck, XCircle } from "lucide-react";
import * as React from "react";

import {
  resolveDecisionDeadlineSecs,
  startPermissionDecisionDelivery,
} from "@/features/agents/lib/permissionDecisionDelivery";
import { formatTranscriptTimestampTitle } from "../agentSessionUtils";
import { ActivityRow, ActivityRowLabel } from "./ActivityRow";
import { ToolActivity } from "./ToolActivity";
import type { ActivityRenderClassItemProps } from "./types";

/**
 * Split the permission item's text into the request description lines and the
 * options line.  The text is newline-joined by describePermissionRequest:
 *   [request title?] [toolCallId?] ["Options: ..."]
 * We surface the options line separately so the render can style it distinctly.
 */
function splitPermissionText(text: string): {
  requestLines: string;
  optionsLine: string | null;
} {
  const lines = text.split("\n");
  const optionsIdx = lines.findIndex((l) => l.startsWith("Options: "));
  if (optionsIdx === -1) {
    return { requestLines: text, optionsLine: null };
  }
  return {
    requestLines: lines.slice(0, optionsIdx).join("\n"),
    optionsLine: lines[optionsIdx],
  };
}

/**
 * Derive the visual tone and icon for a resolved permission outcome string.
 * Outcome strings come from describePermissionOutcome:
 *   "Approved (...)" | "Denied (...)" | "Cancelled" | "uncertain" pinned copy
 */
function permissionOutcomeTone(outcome: string): "approve" | "deny" | "cancel" {
  if (outcome.startsWith("Approved")) return "approve";
  if (outcome.startsWith("Denied")) return "deny";
  return "cancel";
}

/**
 * Exact recognized permission-option kinds the observer feed can act on.
 * Only `allow_once` and `reject_once` are actionable — matching the thread
 * card's two-option contract. `reject_always` is deliberately excluded: the
 * read loop accepts only the two snapshotted ruled IDs (allow_once/reject_once),
 * so a `reject_always` click would be sent, acknowledged as "sent", but silently
 * ignored by the loop — the request would stay pending until timeout with no
 * persistent denial installed. Any kind not in this set is treated as unknown
 * and rendered non-actionable.
 */
const ACTIONABLE_KINDS = new Set(["allow_once", "reject_once"]);

function isActionableKind(kind: string): boolean {
  return ACTIONABLE_KINDS.has(kind);
}

/**
 * Default button label when the harness omits one.
 */
function defaultOptionLabel(kind: string): string {
  if (kind === "reject_once") return "Deny";
  return "Allow";
}

/**
 * Allow/Deny buttons for an actionable permission card.
 * Renders the agent's exact options as labeled buttons; a click sends the
 * `permission_decision` control event (fire-and-forget).
 *
 * On authoritative delivery failure (`no_active_turn`, `channel_closed`,
 * `no_channel`), buttons are re-enabled so the user can retry. The transient
 * `channel_full` status does NOT re-enable buttons — the retransmit
 * orchestrator handles that status automatically and keeps resending. The
 * harness's 300 s fail-closed timeout is the backstop for permanently lost
 * frames.
 */
function PermissionDecisionButtons({
  agentPubkey,
  channelId,
  options,
  requestNonce,
  deliveryFailed,
  deadlineSecs,
  _deliveryFn,
}: {
  agentPubkey: string;
  channelId: string;
  options: Array<{ optionId: string; kind: string; label?: string }>;
  requestNonce: string;
  /**
   * Monotonically increasing failure token from the reducer — incremented on
   * every authoritative delivery failure (`no_active_turn`, `channel_closed`,
   * `no_channel`). The transient `channel_full` status does NOT increment this
   * token; the retransmit orchestrator handles that status automatically.
   * Keying the effect on this number (not a boolean) ensures a second failure
   * after a retry also re-enables buttons.
   */
  deliveryFailed?: number;
  /**
   * Effective expiry deadline (unix seconds) bounding the retransmit loop.
   */
  deadlineSecs: number;
  /**
   * Seam for testing — injects a mock delivery function without needing
   * `mock.module`. Production callers omit this; the real
   * `startPermissionDecisionDelivery` is used by default.
   */
  _deliveryFn?: typeof startPermissionDecisionDelivery;
}) {
  const deliveryFn = _deliveryFn ?? startPermissionDecisionDelivery;
  const [pending, setPending] = React.useState<string | null>(null);

  // Re-enable buttons when the reducer signals an authoritative delivery
  // failure (`no_active_turn`, `channel_closed`, `no_channel`). The transient
  // `channel_full` status does NOT increment this token — the retransmit
  // orchestrator stays subscribed and keeps resending automatically, so buttons
  // must remain disabled until the retry settles or the deadline expires.
  React.useEffect(() => {
    if (deliveryFailed) {
      setPending(null);
    }
  }, [deliveryFailed]);

  // Classify each option into an actionable bucket or a non-actionable
  // display-only slot.  Recognition is an EXACT allowlist, never a prefix:
  // an unknown kind (including an unrecognized `reject_*` such as
  // `reject_later_v2`) fails closed and is not rendered, so the user cannot
  // click a trusted-looking button whose semantics this UI doesn't understand.
  const actionableOptions = options.filter(({ kind }) =>
    isActionableKind(kind),
  );

  if (actionableOptions.length === 0) {
    return null;
  }

  return (
    <div className="mt-1.5 flex flex-wrap gap-1.5">
      {actionableOptions.map(({ optionId, kind, label }) => {
        const isDeny = kind === "reject_once";
        const displayLabel = label ?? defaultOptionLabel(kind);
        return (
          <button
            key={optionId}
            type="button"
            className={
              isDeny
                ? "rounded px-2 py-0.5 text-xs font-medium border border-destructive/40 text-destructive hover:bg-destructive/10 disabled:opacity-50"
                : "rounded px-2 py-0.5 text-xs font-medium border border-green-600/40 text-green-700 dark:text-green-400 hover:bg-green-600/10 disabled:opacity-50"
            }
            data-testid={`permission-decision-${optionId}`}
            disabled={pending !== null}
            onClick={() => {
              setPending(optionId);
              void deliveryFn({
                agentPubkey,
                channelId,
                requestNonce,
                optionId,
                deadlineSecs,
              })
                .then((outcome) => {
                  // `"failed"` means the harness received the frame but could
                  // not route it — re-enable so the user can retry. The reducer
                  // `deliveryFailed` path also re-enables via the `control_result`
                  // frame; this fast path handles the case before the reducer
                  // fires. `"acked"` / `"expired"` are terminal; the transcript
                  // item updates via the observer relay and no retry is needed.
                  if (outcome === "failed") setPending(null);
                })
                .catch(() => {
                  // The delivery loop never rejects — it resolves one of
                  // "acked" | "expired" | "failed". This branch guards against
                  // any unexpected error and re-enables for safety.
                  setPending(null);
                });
            }}
          >
            {pending === optionId ? "…" : displayLabel}
          </button>
        );
      })}
    </div>
  );
}

export function LifecycleActivity(
  props: ActivityRenderClassItemProps & {
    /**
     * Seam for testing — injected mock delivery function threaded through to
     * `PermissionDecisionButtons`. Production callers omit this prop.
     */
    _deliveryFn?: typeof startPermissionDecisionDelivery;
  },
) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "lifecycle") {
    return null;
  }

  const isError =
    props.item.renderClass === "error" ||
    props.item.title.toLowerCase().includes("error");
  const isPermission = props.item.renderClass === "permission";
  const timestampTitle = formatTranscriptTimestampTitle(props.item.timestamp);

  if (isPermission) {
    const { requestLines, optionsLine } = splitPermissionText(props.item.text);
    const outcome = props.item.outcome;
    const tone = outcome ? permissionOutcomeTone(outcome) : null;
    const actionable = props.item.actionable ?? false;
    const requestNonce = props.item.requestNonce;
    const options = props.item.options ?? [];
    const authorizationReason = props.item.authorizationReason;
    const deliveryFailed = props.item.deliveryFailed;
    return (
      <div
        className="rounded-md border border-amber-500/20 bg-amber-500/5 px-2 py-1.5 text-left text-xs text-amber-700 dark:text-amber-400"
        data-testid="transcript-permission-item"
        title={timestampTitle}
      >
        {/* Row 1: request */}
        <div>
          <ShieldCheck className="mr-1.5 inline h-3.5 w-3.5 align-text-bottom" />
          <span className="font-medium">{props.item.title}</span>
          {requestLines ? (
            <span className="opacity-80"> · {requestLines}</span>
          ) : null}
        </div>
        {/* Row 2: authorization reason (from envelope), if present */}
        {authorizationReason ? (
          <div className="mt-0.5 pl-5 opacity-70">{authorizationReason}</div>
        ) : null}
        {/* Row 3: options sub-line (legacy fallback) */}
        {optionsLine && !authorizationReason ? (
          <div className="mt-0.5 pl-5 opacity-60">{optionsLine}</div>
        ) : null}
        {/* Row 4: Allow/Deny buttons (actionable card awaiting decision) */}
        {actionable && requestNonce && !outcome ? (
          <PermissionDecisionButtons
            agentPubkey={props.agentPubkey}
            channelId={props.item.channelId ?? ""}
            options={options}
            requestNonce={requestNonce}
            deliveryFailed={deliveryFailed}
            deadlineSecs={resolveDecisionDeadlineSecs(
              props.item.expiresAt,
              props.item.timestamp,
              Date.now() / 1000,
            )}
            _deliveryFn={props._deliveryFn}
          />
        ) : null}
        {/* Row 5: decision — only when outcome is resolved */}
        {outcome && tone ? (
          <>
            <div className="my-1 border-t border-amber-500/20" />
            <div
              className={
                tone === "approve"
                  ? "flex items-center gap-1 font-medium text-green-600 dark:text-green-400"
                  : tone === "deny"
                    ? "flex items-center gap-1 font-medium text-destructive"
                    : "flex items-center gap-1 font-medium text-muted-foreground"
              }
              data-testid="transcript-permission-outcome"
            >
              {tone === "approve" ? (
                <CheckCircle2 className="h-3.5 w-3.5 shrink-0" />
              ) : tone === "deny" ? (
                <XCircle className="h-3.5 w-3.5 shrink-0" />
              ) : (
                <XCircle className="h-3.5 w-3.5 shrink-0 opacity-50" />
              )}
              {outcome}
            </div>
          </>
        ) : null}
      </div>
    );
  }

  if (isError) {
    return (
      <div
        className="rounded-md border border-destructive/20 bg-destructive/5 px-2 py-1.5 text-left text-xs text-destructive"
        data-testid="transcript-lifecycle-item"
        title={timestampTitle}
      >
        <AlertCircle className="mr-1.5 inline h-3.5 w-3.5 align-text-bottom" />
        <span className="font-medium">{props.item.title}</span>
        {props.item.text ? (
          <span className="opacity-80"> · {props.item.text}</span>
        ) : null}
      </div>
    );
  }

  return (
    <ActivityRow testId="transcript-lifecycle-item" title={timestampTitle}>
      <ActivityRowLabel
        object={props.item.text || undefined}
        openToneScope="none"
        verb={props.item.title}
      />
    </ActivityRow>
  );
}
