import { AlertCircle, CheckCircle2, ShieldCheck, XCircle } from "lucide-react";
import * as React from "react";

import { sendPermissionDecision } from "@/shared/api/agentControl";
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
 * Allow/Deny buttons for an actionable permission card.
 * Renders the agent's exact options as labeled buttons; a click sends the
 * `permission_decision` control event (fire-and-forget).
 *
 * On send failure (relay reject or non-`sent` delivery status), buttons are
 * re-enabled so the user can retry. The harness's 300 s fail-closed timeout
 * is the backstop for permanently lost frames.
 */
function PermissionDecisionButtons({
  agentPubkey,
  channelId,
  options,
  requestNonce,
  deliveryFailed,
}: {
  agentPubkey: string;
  channelId: string;
  options: Array<{ optionId: string; kind: string; label?: string }>;
  requestNonce: string;
  /**
   * Monotonically increasing failure token from the reducer — incremented on
   * every non-`sent` `control_result`. Keying the effect on this number (not a
   * boolean) ensures a second failure after a retry also re-enables buttons.
   */
  deliveryFailed?: number;
}) {
  const [pending, setPending] = React.useState<string | null>(null);

  // Re-enable buttons when the reducer signals delivery failure (non-`sent`
  // control_result status). The relay send succeeded but the harness couldn't
  // route the click — the user should be able to retry.
  React.useEffect(() => {
    if (deliveryFailed) {
      setPending(null);
    }
  }, [deliveryFailed]);

  if (options.length === 0) {
    return null;
  }

  return (
    <div className="mt-1.5 flex flex-wrap gap-1.5">
      {options.map(({ optionId, kind, label }) => {
        const isDeny = kind.startsWith("reject");
        const displayLabel = label ?? (isDeny ? "Deny" : "Allow");
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
              void sendPermissionDecision(
                agentPubkey,
                channelId,
                requestNonce,
                optionId,
              ).catch(() => {
                // Relay rejected the send. Re-enable so the user can retry;
                // the harness's 300 s fail-closed timeout handles permanent loss.
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

export function LifecycleActivity(props: ActivityRenderClassItemProps) {
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
