import * as React from "react";
import { AlertCircle, CheckCircle2, ShieldCheck, XCircle } from "lucide-react";
import { toast } from "sonner";

import { resolveManagedAgentPermission } from "@/shared/api/agentControl";
import { Button } from "@/shared/ui/button";
import { oneShotPermissionOptions } from "../agentSessionPermission";
import type { AgentPermissionOption } from "../agentSessionTypes";
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
 *   "Approved (...)" | "Denied (...)" | "Cancelled"
 */
function permissionOutcomeTone(outcome: string): "approve" | "deny" | "cancel" {
  if (outcome.startsWith("Approved")) return "approve";
  if (outcome.startsWith("Denied")) return "deny";
  return "cancel";
}

function PermissionDecisionActions({
  agentPubkey,
  channelId,
  requestId,
  turnId,
  options,
}: {
  agentPubkey: string;
  channelId: string;
  requestId: string | number;
  turnId: string;
  options: AgentPermissionOption[];
}) {
  const [submitting, setSubmitting] = React.useState<string | null>(null);

  const decide = React.useCallback(
    async (optionId: string) => {
      setSubmitting(optionId);
      try {
        await resolveManagedAgentPermission(
          agentPubkey,
          channelId,
          turnId,
          requestId,
          optionId,
        );
      } catch {
        setSubmitting(null);
        toast.error("Couldn’t send the permission decision. Try again.");
      }
    },
    [agentPubkey, channelId, requestId, turnId],
  );

  const oneShotOptions = oneShotPermissionOptions(options);
  const allowOnce = oneShotOptions.find(
    (option) => option.kind === "allow_once",
  );
  const rejectOnce = oneShotOptions.find(
    (option) => option.kind === "reject_once",
  );

  return (
    <>
      <div className="my-1.5 border-t border-amber-500/20" />
      <fieldset
        className="flex items-center gap-2"
        data-testid="transcript-permission-actions"
      >
        <legend className="sr-only">Permission decision</legend>
        {allowOnce ? (
          <Button
            disabled={submitting !== null}
            onClick={() => void decide(allowOnce.optionId)}
            size="xs"
            type="button"
          >
            {submitting === allowOnce.optionId ? "Allowing…" : allowOnce.name}
          </Button>
        ) : null}
        {rejectOnce ? (
          <Button
            disabled={submitting !== null}
            onClick={() => void decide(rejectOnce.optionId)}
            size="xs"
            type="button"
            variant="outline"
          >
            {submitting === rejectOnce.optionId ? "Denying…" : rejectOnce.name}
          </Button>
        ) : null}
      </fieldset>
    </>
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
        {/* Row 2: options (muted sub-line) */}
        {optionsLine ? (
          <div className="mt-0.5 pl-5 opacity-60">{optionsLine}</div>
        ) : null}
        {/* Row 3: decision — only when outcome is resolved */}
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
        ) : props.item.permissionRequestId != null &&
          props.item.channelId &&
          props.item.turnId &&
          props.item.permissionOptions?.length ? (
          <PermissionDecisionActions
            agentPubkey={props.agentPubkey}
            channelId={props.item.channelId}
            requestId={props.item.permissionRequestId}
            turnId={props.item.turnId}
            options={props.item.permissionOptions ?? []}
          />
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
