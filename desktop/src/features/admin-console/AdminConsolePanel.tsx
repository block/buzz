/**
 * Main admin console panel — renders when probe state is `nip98Authorized` or
 * `disabled`.
 *
 * Shows three tabs: Reports (deployment-wide moderation reports), Feedback
 * (product feedback with optional image attachments), and Staffing (Operator-
 * only operator management).
 *
 * All query/UI state is keyed by `(pubkey, origin)`. In-flight native requests
 * are fenced by an effect-local `active` flag that is set to `false` in the
 * effect cleanup, ensuring stale results are discarded on arrival.
 *
 * Tauri invoke is not cancellable at the native layer, but the active-flag
 * pattern ensures stale results never update visible state or create
 * unreachable blob URLs.
 *
 * Sub-components live in adjacent files:
 *   - AdminConsolePanelHelpers.tsx  — AsyncState, useAsyncLoad, formatTimestamp,
 *                                     DetailRow, LoadingSpinner, ErrorMessage,
 *                                     AttachmentMeta, parseImetaAttachments
 *   - AdminConsoleFeedbackTab.tsx   — FeedbackTab, FeedbackDetail
 *   - AdminConsoleStaffingTab.tsx   — StaffingTab
 */

import { useEffect, useRef, useState } from "react";
import {
  ChevronLeft,
  LoaderCircle,
  MessageSquare,
  ShieldAlert,
  Users,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";
import {
  getAdminReport,
  listAdminReports,
  cancelAdminReport,
  reopenAdminReport,
  resolveAdminReport,
  type AdminPrincipalRole,
  type AdminReportAction,
  type AdminReportDetailDto,
  type AdminReportDto,
  type AdminReportResolution,
} from "./api";
import {
  DetailRow,
  ErrorMessage,
  LoadingSpinner,
  CommunityGroupedList,
  formatTimestamp,
  useAsyncLoad,
  adminErrorMessage,
  preserveRequestIdOnError,
} from "./AdminConsolePanelHelpers";
import { FeedbackTab } from "./AdminConsoleFeedbackTab";
import { StaffingTab } from "./AdminConsoleStaffingTab";

export {
  parseImetaAttachments,
  type AttachmentMeta,
} from "./AdminConsolePanelHelpers";

// ── Status variant helper ─────────────────────────────────────────────────

function statusVariant(
  status: string,
): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "open":
      return "default";
    case "resolved":
      return "secondary";
    case "dismissed":
      return "outline";
    case "escalated":
      return "secondary";
    case "processing":
      return "secondary";
    case "pending":
      return "secondary";
    case "enforcing":
      return "secondary";
    case "succeeded":
      return "secondary";
    case "failed":
      return "destructive";
    case "cancelled":
      return "outline";
    default:
      return "outline";
  }
}

// ── Action matrix helpers ─────────────────────────────────────────────────

/**
 * Return the allowed actions for a given target kind per the v4 frozen matrix.
 * event:  delete | kick | ban | timeout | dismiss | escalate
 * pubkey: ban | timeout | dismiss | escalate
 * blob:   dismiss | escalate
 */
function allowedActionsForTargetKind(targetKind: string): AdminReportAction[] {
  switch (targetKind.toLowerCase()) {
    case "event":
      return ["delete", "kick", "ban", "timeout", "dismiss", "escalate"];
    case "pubkey":
      return ["ban", "timeout", "dismiss", "escalate"];
    case "blob":
      return ["dismiss", "escalate"];
    default:
      return ["dismiss", "escalate"];
  }
}

/** Label for each action. */
function actionLabel(action: AdminReportAction): string {
  switch (action) {
    case "delete":
      return "Delete";
    case "kick":
      return "Kick";
    case "ban":
      return "Ban";
    case "timeout":
      return "Timeout";
    case "dismiss":
      return "Dismiss";
    case "escalate":
      return "Escalate";
  }
}

/** Variant for each action button. */
function actionVariant(
  action: AdminReportAction,
): "destructive" | "outline" | "secondary" {
  switch (action) {
    case "delete":
    case "ban":
      return "destructive";
    case "kick":
    case "timeout":
      return "outline";
    default:
      return "secondary";
  }
}

// ── Enforcement state block ───────────────────────────────────────────────

/**
 * Inline enforcement-state block shown on `processing` reports and after a
 * failed enforcement action. Shows the action record's state and offers cancel
 * on a `failed` action.
 *
 * A `failed` action is always pre-mutation (the relay only records `failed`
 * before the enforcement side effect lands), so it is always cancellable.
 * Cancel is the only recovery path: it returns the report to `open` for a
 * fresh resolution. There is no client-side "retry" — composing cancel + a new
 * resolve would imply an atomicity the relay does not provide, leaving a window
 * where the report is open with no explanation if the second call is lost.
 *
 * `pending`/`enforcing` actions are NOT cancellable over HTTP — the relay's
 * recovery worker owns their convergence — so no button is offered there.
 * A rejected cancel (409) is authoritative: the action already advanced or
 * someone else cancelled it, so reload detail rather than retrying.
 */
function EnforcementStateBlock({
  activeAction,
  canMutate,
  origin,
  reportId,
  onActionComplete,
}: {
  activeAction: NonNullable<AdminReportDto["activeAction"]>;
  /** Whether mutation controls are enabled. `false` in disabled-auth mode. */
  canMutate: boolean;
  origin: string;
  reportId: string;
  onActionComplete: () => void;
}) {
  const [isWorking, setIsWorking] = useState(false);

  const actionStatus = activeAction.status;

  // User-facing copy for each action state.
  const stateLabel: Record<string, string> = {
    pending: "Enforcement pending…",
    enforcing: "Enforcing…",
    succeeded: "Enforcement succeeded",
    failed: "Enforcement failed",
    cancelled: "Enforcement cancelled",
  };

  const handleCancel = async () => {
    setIsWorking(true);
    try {
      // Fence the cancel to the exact failed action the operator observed. On
      // success the report returns to `open`; the detail reload then serves
      // `activeAction: null` and re-exposes the resolve form for a fresh attempt.
      await cancelAdminReport(origin, reportId, {
        actionId: activeAction.id,
      });
      toast.success("Enforcement cancelled — report reopened");
      onActionComplete();
    } catch (e) {
      // A 409 means the action is no longer cancellable (already cancelled,
      // superseded, or past the mutation point). Reload detail rather than
      // retry — the toast is informational, the reload shows current state.
      toast.error(`Cancel rejected: ${adminErrorMessage(e)}`);
      onActionComplete();
    } finally {
      setIsWorking(false);
    }
  };

  return (
    <div
      className="rounded-md border border-border/60 px-3 py-2.5 space-y-2"
      data-testid="enforcement-state-block"
    >
      <div className="flex items-center gap-2 text-sm">
        {(actionStatus === "pending" || actionStatus === "enforcing") && (
          <LoaderCircle className="h-4 w-4 animate-spin text-muted-foreground" />
        )}
        <Badge variant={statusVariant(actionStatus)}>
          {stateLabel[actionStatus] ?? actionStatus}
        </Badge>
        <span className="text-xs text-muted-foreground font-mono">
          {activeAction.action}
        </span>
      </div>
      {activeAction.errorMessage && (
        <p className="text-xs text-muted-foreground break-words">
          {activeAction.errorMessage}
        </p>
      )}
      {actionStatus === "failed" && canMutate && (
        <Button
          data-testid="enforcement-cancel-btn"
          disabled={isWorking}
          onClick={() => void handleCancel()}
          size="sm"
          type="button"
          variant="outline"
        >
          {isWorking ? (
            <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
          ) : (
            "Cancel & reopen"
          )}
        </Button>
      )}
    </div>
  );
}

// ── Resolve report form ───────────────────────────────────────────────────

/**
 * Disclose where the reason travels for each action family so operators
 * understand the privacy and public-notice implications before submitting.
 *
 * - delete: verbatim to the affected user AND publicly in the room tombstone.
 * - kick / ban / timeout: verbatim to the affected user only.
 * - dismiss / escalate: verbatim to the reporter (no affected-user notice).
 *
 * Source: relay_admin_actions.rs and admin_outbox_worker.rs notice paths.
 */
function reasonAudienceCopy(action: AdminReportAction): string {
  switch (action) {
    case "delete":
      return "Sent verbatim to the affected user and posted publicly in the room.";
    case "kick":
    case "ban":
    case "timeout":
      return "Sent verbatim to the affected user.";
    case "dismiss":
    case "escalate":
      return "Sent verbatim to the reporter.";
  }
}

/**
 * Derive a human-readable action label from an `AdminReportResolution`.
 *
 * `activeAction.action` is authoritative for enforcement actions. For
 * decision-only resolutions (dismiss/escalate), `activeAction` is null;
 * the terminal `status` encodes the outcome. Never falls back to form state.
 */
function resolutionLabel(resolution: AdminReportResolution): string {
  if (resolution.activeAction?.action) {
    return actionLabel(resolution.activeAction.action);
  }
  switch (resolution.status) {
    case "dismissed":
      return actionLabel("dismiss");
    case "escalated":
      return actionLabel("escalate");
    default:
      return resolution.status;
  }
}

/**
 * Frozen submit payload — the whole command sent on first attempt. Retained
 * across ambiguous failures for byte-for-byte retry; cleared only on a
 * definitive pre-commit rejection (non-409 4xx with full body).
 */
type FrozenPayload = {
  requestId: string;
  action: AdminReportAction;
  reason: string | undefined;
  expirationSecs: number | undefined;
};

/**
 * Resolution form — shown on open reports (not `processing`). Presents the
 * action matrix for the report's target_kind, collects optional reason and
 * (for timeout) expiration_secs, then calls the resolve endpoint.
 *
 * The form generates a `requestId` per submission attempt. On retry after a
 * lost response, the caller should reuse the same `requestId` — this is
 * handled by the retry path in `EnforcementStateBlock`.
 */
function ResolveReportForm({
  report,
  origin,
  onResolved,
}: {
  report: AdminReportDto;
  origin: string;
  onResolved: () => void;
}) {
  const [selectedAction, setSelectedAction] =
    useState<AdminReportAction | null>(null);
  const [reason, setReason] = useState("");
  const [expirationSecs, setExpirationSecs] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  // Frozen whole-payload snapshot. Set on first submit; retained across
  // ambiguous failures; cleared on a definitive pre-commit rejection.
  const frozenRef = useRef<FrozenPayload | null>(null);

  // Locked between attempts: snapshot held but not actively submitting.
  // Prevents edits that would diverge from the frozen idempotency payload.
  const isLocked = frozenRef.current !== null && !isSubmitting;

  // Kick removes the target from the report's associated channel, so the
  // relay rejects it (400 invalid_action_for_target) when the report carries
  // no channel. Suppress it client-side rather than offer a guaranteed failure.
  const allowedActions = allowedActionsForTargetKind(
    report.targetKind ?? "",
  ).filter((a) => a !== "kick" || report.channelId != null);

  const handleSubmit = async () => {
    if (!selectedAction) return;
    setIsSubmitting(true);

    // On first attempt freeze the whole payload; on retry reuse it byte-for-byte.
    if (!frozenRef.current) {
      frozenRef.current = {
        requestId: crypto.randomUUID(),
        action: selectedAction,
        reason: reason.trim() || undefined,
        expirationSecs:
          selectedAction === "timeout" && expirationSecs
            ? Number(expirationSecs)
            : undefined,
      };
    }
    const payload = frozenRef.current;

    try {
      const resolution = await resolveAdminReport(origin, report.id, {
        action: payload.action,
        requestId: payload.requestId,
        expirationSecs: payload.expirationSecs,
        reason: payload.reason,
      });
      // Derive toast from the authoritative relay response, not mutable form
      // state — relay idempotency executes the first command even on retry.
      toast.success(`Report resolved: ${resolutionLabel(resolution)}`);
      onResolved();
    } catch (e) {
      // Preserve the frozen payload whenever the outcome is ambiguous (409,
      // 5xx, a lost response, or a transport failure with no relay answer)
      // so a retry reuses the same idempotency key and the relay dedupes.
      // Discard only on a definitive pre-commit rejection (a non-409 4xx with
      // full body), where a corrected resubmission is a genuinely new command.
      if (!preserveRequestIdOnError(e)) {
        frozenRef.current = null;
      }
      toast.error(adminErrorMessage(e));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div
      className="rounded-md border border-border/60 px-3 py-2.5 space-y-3"
      data-testid="resolve-report-form"
    >
      <p className="text-xs font-medium text-muted-foreground">
        Resolve report
      </p>
      <div className="flex flex-wrap gap-1.5">
        {allowedActions.map((action) => (
          <Button
            className={cn(
              "text-xs",
              selectedAction === action && "ring-2 ring-ring ring-offset-1",
            )}
            data-testid={`action-btn-${action}`}
            disabled={isLocked || isSubmitting}
            key={action}
            onClick={() =>
              setSelectedAction(action === selectedAction ? null : action)
            }
            size="sm"
            type="button"
            variant={actionVariant(action)}
          >
            {actionLabel(action)}
          </Button>
        ))}
      </div>

      {selectedAction === "timeout" && (
        <div className="flex items-center gap-2">
          <label
            className="text-xs text-muted-foreground shrink-0"
            htmlFor="timeout-duration-input"
          >
            Duration (seconds)
          </label>
          <input
            className="flex-1 rounded-md border border-border/60 bg-background px-2 py-1 text-xs font-mono"
            data-testid="timeout-duration-input"
            disabled={isLocked || isSubmitting}
            id="timeout-duration-input"
            min="1"
            onChange={(e) => setExpirationSecs(e.target.value)}
            placeholder="e.g. 3600"
            type="number"
            value={expirationSecs}
          />
        </div>
      )}

      <div>
        <input
          className="w-full rounded-md border border-border/60 bg-background px-2 py-1 text-xs"
          data-testid="resolve-reason-input"
          disabled={isLocked || isSubmitting}
          onChange={(e) => setReason(e.target.value)}
          placeholder="Reason (optional)"
          type="text"
          value={reason}
        />
        {selectedAction && (
          <p
            className="mt-1 text-xs text-muted-foreground"
            data-testid="resolve-reason-audience"
          >
            {reasonAudienceCopy(selectedAction)}
          </p>
        )}
      </div>

      {selectedAction && (
        <Button
          data-testid="resolve-submit-btn"
          disabled={
            isSubmitting ||
            (selectedAction === "timeout" &&
              !expirationSecs &&
              !frozenRef.current)
          }
          onClick={() => void handleSubmit()}
          size="sm"
          type="button"
        >
          {isSubmitting ? (
            <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
          ) : (
            `Confirm: ${actionLabel(frozenRef.current?.action ?? selectedAction)}`
          )}
        </Button>
      )}
    </div>
  );
}

// ── Reopen report form ────────────────────────────────────────────────────

/**
 * Reopen form — shown on terminal reports (`resolved` | `dismissed` |
 * `escalated`). Moves the report back to `open` for re-triage.
 *
 * Reopen is re-triage only: it does NOT reverse any enforcement already taken
 * (no un-ban, no un-timeout, no message restore). The copy states this
 * explicitly, and more emphatically when the report carries an `actionId`
 * (an enforcement action was applied while it was resolved).
 *
 * A `requestId` is generated per attempt and reused on retry so a lost
 * response is idempotent, mirroring the resolve flow. A 409 (report not
 * reopenable — e.g. it moved to `processing`) preserves the `requestId`.
 */
function ReopenReportForm({
  report,
  origin,
  onReopened,
}: {
  report: AdminReportDto;
  origin: string;
  onReopened: () => void;
}) {
  const [reason, setReason] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  // Stable requestId per attempt; reused on retry after a lost response.
  const requestIdRef = useRef<string | null>(null);

  // An enforcement action was applied while this report was resolved.
  const wasEnforced = report.actionId != null;

  const handleSubmit = async () => {
    setIsSubmitting(true);

    if (!requestIdRef.current) {
      requestIdRef.current = crypto.randomUUID();
    }

    try {
      await reopenAdminReport(origin, report.id, {
        requestId: requestIdRef.current,
        reason: reason.trim() || undefined,
      });
      toast.success("Report reopened");
      onReopened();
    } catch (e) {
      // Preserve the requestId on an ambiguous outcome (409, 5xx, lost
      // response, or a transport failure with no relay answer) so a retry
      // reuses the same idempotency key; reset only on a definitive pre-commit
      // rejection (a non-409 4xx).
      if (!preserveRequestIdOnError(e)) {
        requestIdRef.current = null;
      }
      toast.error(adminErrorMessage(e));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div
      className="rounded-md border border-border/60 px-3 py-2.5 space-y-3"
      data-testid="reopen-report-form"
    >
      <div className="space-y-1">
        <p className="text-xs font-medium text-muted-foreground">
          Reopen report
        </p>
        <p className="text-xs text-muted-foreground">
          Moves this report back to the open queue for re-triage.{" "}
          {wasEnforced
            ? "The enforcement action already taken is not reversed — reopening does not un-ban, un-timeout, or restore a deleted message."
            : "Reopening does not reverse any enforcement action."}
        </p>
      </div>

      <input
        className="w-full rounded-md border border-border/60 bg-background px-2 py-1 text-xs"
        data-testid="reopen-reason-input"
        onChange={(e) => setReason(e.target.value)}
        placeholder="Reason (optional)"
        type="text"
        value={reason}
      />

      <Button
        data-testid="reopen-submit-btn"
        disabled={isSubmitting}
        onClick={() => void handleSubmit()}
        size="sm"
        type="button"
        variant="outline"
      >
        {isSubmitting ? (
          <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
        ) : (
          "Reopen report"
        )}
      </Button>
    </div>
  );
}

// ── Reports tab ───────────────────────────────────────────────────────────

function ReportsTab({
  canMutate,
  origin,
  pubkey,
  generation,
}: {
  canMutate: boolean;
  origin: string;
  pubkey: string;
  generation: number;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // List refresh fence: bumped whenever a mutation completes in the detail
  // view, so returning to the list shows fresh status without a tab switch.
  const [listGen, setListGen] = useState(0);

  const listState = useAsyncLoad(
    // Request the full workflow queue: open, processing, resolved, dismissed,
    // escalated. The relay's omitted-scope default is escalated-only (the
    // platform-safety backstop); scope=all gives this console access to the
    // states its own resolve/cancel/reopen controls act on.
    () => listAdminReports(origin, { scope: "all" }),
    [origin, pubkey],
    generation + listGen,
  );

  if (selectedId) {
    return (
      <ReportDetail
        canMutate={canMutate}
        origin={origin}
        pubkey={pubkey}
        generation={generation}
        reportId={selectedId}
        onBack={() => setSelectedId(null)}
        onMutated={() => setListGen((g) => g + 1)}
      />
    );
  }

  if (listState.status === "loading") {
    return <LoadingSpinner />;
  }
  if (listState.status === "error") {
    return <ErrorMessage message={listState.message} />;
  }
  if (listState.status !== "ok") return null;

  const reports = listState.data;
  if (!Array.isArray(reports) || reports.length === 0) {
    return <p className="text-sm text-muted-foreground">No reports found.</p>;
  }

  return (
    <CommunityGroupedList
      items={reports}
      renderItem={(report: AdminReportDto) => {
        const id = report.id;
        const summary = report.reportType || "Report";
        const status = report.status;
        const isProcessing = status === "processing";
        return (
          <li key={id}>
            {/* Processing rows stay navigable: the enforcement state (progress,
                retry, cancel) lives inside the detail view, so disabling the row
                would hide exactly the controls an operator needs while an action
                is pending. Detail suppresses only the resolve form for a
                non-open report. */}
            <button
              className="w-full rounded-md border border-border/60 px-3 py-2.5 text-left text-sm hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => setSelectedId(id)}
              type="button"
            >
              <span className="block font-medium">{summary}</span>
              {status && (
                <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                  {status}
                  {isProcessing && (
                    <LoaderCircle className="h-3 w-3 animate-spin" />
                  )}
                </span>
              )}
            </button>
          </li>
        );
      }}
    />
  );
}

function ReportFields({ data }: { data: AdminReportDetailDto }) {
  const status = data.status ?? "";
  return (
    <div
      className="space-y-1.5 rounded-md border border-border/60 px-3 py-2.5"
      data-testid="report-detail-fields"
    >
      <div className="mb-2 flex flex-wrap items-center gap-2">
        {status && <Badge variant={statusVariant(status)}>{status}</Badge>}
        {data.reportType && <Badge variant="outline">{data.reportType}</Badge>}
      </div>
      <DetailRow label="ID" value={data.id} mono />
      <DetailRow label="Community" value={data.communityId} mono />
      <DetailRow label="Host" value={data.communityHost} />
      <DetailRow label="Event ID" value={data.reportEventId} mono />
      <DetailRow label="Reporter" value={data.reporterPubkey} mono />
      <DetailRow label="Target kind" value={data.targetKind} />
      <DetailRow label="Target" value={data.target} mono />
      <DetailRow label="Channel" value={data.channelId ?? null} mono />
      <DetailRow label="Note" value={data.note ?? null} />
      <DetailRow label="Resolved by" value={data.resolvedBy ?? null} mono />
      <DetailRow label="Resolved at" value={formatTimestamp(data.resolvedAt)} />
      <DetailRow label="Action ID" value={data.actionId ?? null} mono />
      <DetailRow label="Created" value={formatTimestamp(data.createdAt)} />
      {data.message != null && (
        <div className="mt-3 space-y-1.5 rounded-md border border-border/40 px-3 py-2.5">
          <p className="mb-1 text-xs font-semibold text-muted-foreground">
            Reported message
            {data.message.deletedAt != null && (
              <span className="ml-1.5 text-destructive">(deleted)</span>
            )}
          </p>
          <DetailRow label="Author" value={data.message.authorPubkey} mono />
          <DetailRow label="Content" value={data.message.content} />
          <DetailRow
            label="Msg created"
            value={formatTimestamp(data.message.createdAt)}
          />
        </div>
      )}
    </div>
  );
}

function ReportDetail({
  canMutate,
  origin,
  pubkey,
  generation,
  reportId,
  onBack,
  onMutated,
}: {
  canMutate: boolean;
  origin: string;
  pubkey: string;
  generation: number;
  reportId: string;
  onBack: () => void;
  /** Called after any mutation completes so the parent list can refetch. */
  onMutated: () => void;
}) {
  // Resolution generation: bump to reload detail after an action completes.
  const [resolveGen, setResolveGen] = useState(0);

  // Reload the detail AND signal the parent list on every completed mutation,
  // so back-nav shows fresh status without the tab-switch workaround.
  const handleMutated = () => {
    setResolveGen((g) => g + 1);
    onMutated();
  };

  const detailState = useAsyncLoad(
    () => getAdminReport(origin, reportId),
    [origin, pubkey, reportId],
    generation + resolveGen,
  );

  const data = detailState.status === "ok" ? detailState.data : null;
  const isOpen = data?.status === "open";
  const isReopenable =
    data?.status === "resolved" ||
    data?.status === "dismissed" ||
    data?.status === "escalated";
  const activeAction = data?.activeAction ?? null;

  return (
    <div className="space-y-4">
      <button
        className="mb-3 flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
        onClick={onBack}
        type="button"
      >
        <ChevronLeft className="h-3.5 w-3.5" />
        Back to reports
      </button>
      {detailState.status === "loading" && <LoadingSpinner />}
      {detailState.status === "error" && (
        <ErrorMessage message={detailState.message} />
      )}
      {detailState.status === "ok" && (
        <>
          <ReportFields data={detailState.data} />
          {/* Enforcement state / history. The detail LATERAL returns an action
              whenever one governs the report: a live action (pending/enforcing)
              or a cancellable failed action while processing, or a succeeded
              action as executed-enforcement history on a terminal or reopened
              report (honest history — a later dismissal/reopen does not
              un-happen the ban that ran). Cancel is offered only on failed,
              inside the block. A cancelled action never reaches this read. */}
          {activeAction && (
            <EnforcementStateBlock
              activeAction={activeAction}
              canMutate={canMutate}
              origin={origin}
              reportId={reportId}
              onActionComplete={handleMutated}
            />
          )}
          {/* Resolve form: shown for open reports. A reopened-after-enforcement
              report is open yet carries a succeeded activeAction (history);
              the form must still show so the operator can re-triage — the
              enforcement block above renders that history alongside it. Only a
              live/failed action keeps the report `processing` (not open), so
              `isOpen` alone never surfaces the form on an in-flight action. */}
          {isOpen && canMutate && (
            <ResolveReportForm
              report={detailState.data}
              origin={origin}
              onResolved={handleMutated}
            />
          )}
          {/* Reopen form: only for terminal (resolved/dismissed/escalated) reports */}
          {isReopenable && canMutate && (
            <ReopenReportForm
              report={detailState.data}
              origin={origin}
              onReopened={handleMutated}
            />
          )}
        </>
      )}
    </div>
  );
}

// ── Tab bar ───────────────────────────────────────────────────────────────

type Tab = "reports" | "feedback" | "staffing";

function TabBar({
  activeTab,
  onSelect,
  showStaffing,
}: {
  activeTab: Tab;
  onSelect: (tab: Tab) => void;
  showStaffing: boolean;
}) {
  const allTabs: Array<{
    value: Tab;
    label: string;
    Icon: React.ComponentType<{ className?: string }>;
  }> = [
    { value: "reports", label: "Reports", Icon: ShieldAlert },
    { value: "feedback", label: "Feedback", Icon: MessageSquare },
    ...(showStaffing
      ? [{ value: "staffing" as const, label: "Staffing", Icon: Users }]
      : []),
  ];
  return (
    <div className="mb-4 flex gap-1 border-b border-border/60">
      {allTabs.map(({ value, label, Icon }) => (
        <button
          className={cn(
            "flex items-center gap-1.5 border-b-2 px-3 pb-2 pt-1 text-sm font-medium transition-colors",
            activeTab === value
              ? "border-primary text-foreground"
              : "border-transparent text-muted-foreground hover:text-foreground",
          )}
          data-testid={`admin-tab-${value}`}
          key={value}
          onClick={() => onSelect(value)}
          type="button"
        >
          <Icon className="h-3.5 w-3.5" />
          {label}
        </button>
      ))}
    </div>
  );
}

// ── Panel root ────────────────────────────────────────────────────────────

export function AdminConsolePanel({
  canMutate,
  origin,
  pubkey,
  role,
  initialTab,
}: {
  /**
   * Whether mutation controls should be enabled. `false` when the relay probe
   * returned `disabled` — the admin API is accessible without credentials, so
   * the operator can read but must not be offered write affordances that could
   * accidentally mutate the relay without authentication.
   */
  canMutate: boolean;
  origin: string;
  /** Active identity pubkey — all state is keyed on (pubkey, origin). */
  pubkey: string;
  /** Principal role from probe — `"operator"` | `"moderator"` | undefined */
  role?: AdminPrincipalRole | null;
  /**
   * Override the initially active tab. Intended for unit tests that need to
   * land on a specific tab without driving click events through MinimalDocument.
   * Do not pass this prop in production code.
   */
  initialTab?: Tab;
}) {
  const isOperator = role === "operator";
  const [activeTab, setActiveTab] = useState<Tab>(initialTab ?? "reports");
  // Increment whenever the (pubkey, origin) context changes to invalidate all
  // in-flight useAsyncLoad effects via their effect-local `active` flags.
  const generationRef = useRef(0);
  const [generation, setGeneration] = useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey and origin are reactive props — effect fires when either changes to bump the generation fence
  useEffect(() => {
    generationRef.current += 1;
    setGeneration(generationRef.current);
  }, [pubkey, origin]);

  // Reset activeTab to the default when the current tab is no longer visible
  // for the current role (e.g. operator→moderator while Staffing is selected).
  // Guard is written against tab-visibility (the set TabBar would render for
  // this role) rather than a hard-coded role string so it generalises to
  // future tabs without requiring a companion role check here.
  useEffect(() => {
    const visibleTabs = new Set<Tab>([
      "reports",
      "feedback",
      ...(isOperator ? (["staffing"] as Tab[]) : []),
    ]);
    if (!visibleTabs.has(activeTab)) {
      setActiveTab("reports");
    }
  }, [isOperator, activeTab]);

  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-testid="admin-console-panel"
    >
      <TabBar
        activeTab={activeTab}
        onSelect={setActiveTab}
        showStaffing={isOperator}
      />
      {activeTab === "reports" && (
        <div data-testid="reports-tab">
          <ReportsTab
            canMutate={canMutate}
            origin={origin}
            pubkey={pubkey}
            generation={generation}
          />
        </div>
      )}
      {activeTab === "feedback" && (
        <FeedbackTab
          canMutate={canMutate}
          origin={origin}
          pubkey={pubkey}
          generation={generation}
        />
      )}
      {activeTab === "staffing" && isOperator && (
        <StaffingTab
          canMutate={canMutate}
          origin={origin}
          pubkey={pubkey}
          generation={generation}
        />
      )}
    </div>
  );
}
