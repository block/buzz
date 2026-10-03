import { useCallback, useRef, useState } from "react";
import {
  type Mike49AuditReport,
  type Mike49Record,
  mike49RunFixtureAudit,
} from "@/shared/api/mike49Audit";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { SettingsOptionGroup } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

type RunState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "success"; report: Mike49AuditReport }
  | { status: "error"; code: string };

/** Fixed, content-free error codes this command's Rust side can reject
 * with — see `desktop/src-tauri/src/commands/mike49_audit/command.rs`'s
 * `Mike49AuditError`. Any other string is shown as-is but never assumed
 * to carry more detail than a code. */
function describeErrorCode(code: string): string {
  switch (code) {
    case "fixture_build_failed":
      return "The in-memory fixture failed to build.";
    case "pagination_failed":
      return "The fixture's simulated pagination call failed.";
    case "reconciliation_failed":
      return "The run's own record counts did not reconcile — the report was withheld rather than shown partial.";
    default:
      return `Run failed (${code}).`;
  }
}

function RecordRow({ record }: { record: Mike49Record }) {
  const isVerified = record.result === "verified";
  return (
    <div
      className="flex flex-col gap-1 rounded-md border border-border/60 p-3"
      data-testid="mike49-record-row"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-medium">{record.scenario}</span>
        <Badge variant={isVerified ? "success" : "destructive"}>
          {isVerified ? "Verified" : "Verify error"}
        </Badge>
      </div>
      {isVerified ? (
        <p className="text-xs text-muted-foreground/80" data-settings-subcopy>
          outcome: {record.observeOutcomeCode ?? "unknown"}
          {record.sessionId ? ` · session: ${record.sessionId}` : ""}
          {record.turnSeq != null ? ` · turn ${record.turnSeq}` : ""}
        </p>
      ) : (
        <p className="text-xs text-muted-foreground/80" data-settings-subcopy>
          reason: {record.verifyErrorCode ?? "unknown"}
        </p>
      )}
    </div>
  );
}

function ReportView({ report }: { report: Mike49AuditReport }) {
  const { pagination, emptySourceProbe, sanitizer } = report;
  return (
    <div className="flex flex-col gap-4" data-testid="mike49-report">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="info">SIMULATED DATA</Badge>
        <Badge variant="outline">source: {report.dataSource}</Badge>
        <Badge
          variant={pagination.localTraversalComplete ? "success" : "warning"}
          data-testid="mike49-pagination-state"
        >
          {pagination.localTraversalComplete
            ? "Traversal complete"
            : "Incomplete — capped"}
        </Badge>
      </div>

      <p className="text-xs text-muted-foreground/80" data-settings-subcopy>
        Pagination examined {pagination.eventsExamined} event
        {pagination.eventsExamined === 1 ? "" : "s"} and stopped because{" "}
        <code>{pagination.stopReason}</code>. {pagination.note}
      </p>

      {report.records.length === 0 ? (
        <p
          className="rounded-md border border-dashed border-border/60 p-3 text-sm text-muted-foreground"
          data-testid="mike49-records-empty"
        >
          No records were fetched in this run's main (capped) pagination call.
        </p>
      ) : (
        <div className="flex flex-col gap-2">
          {report.records.map((record) => (
            <RecordRow
              key={`${record.scenario}-${record.eventId}`}
              record={record}
            />
          ))}
        </div>
      )}

      <div
        className="rounded-md border border-dashed border-border/60 p-3"
        data-testid="mike49-empty-probe"
      >
        <p className="text-sm font-medium">Empty page source probe</p>
        <p className="text-xs text-muted-foreground/80" data-settings-subcopy>
          A separate, genuinely empty fixture source examined{" "}
          {emptySourceProbe.eventsExamined} events (stop reason:{" "}
          <code>{emptySourceProbe.stopReason}</code>). {emptySourceProbe.note}
        </p>
      </div>

      <p className="text-2xs text-muted-foreground/60" data-settings-subcopy>
        sanitizer: {sanitizer.blockedCount} blocked,{" "}
        {sanitizer.droppedFieldCount} unrecognized field
        {sanitizer.droppedFieldCount === 1 ? "" : "s"} dropped
      </p>
    </div>
  );
}

export function Mike49AuditFixturePanel() {
  const [state, setState] = useState<RunState>({ status: "idle" });
  // A synchronous guard against starting a second run while one is already
  // active. `state.status === "loading"` alone is enough to disable the
  // button, but a ref guard is the actual dispatch gate: React state
  // updates are asynchronous/batched, so two rapid clicks could otherwise
  // both read the pre-update "idle" status before either re-render lands.
  const isRunningRef = useRef(false);

  // Explicit run action only — nothing invokes the command on mount.
  const onClick = useCallback(() => {
    if (isRunningRef.current) {
      return;
    }
    isRunningRef.current = true;
    setState({ status: "loading" });
    mike49RunFixtureAudit()
      .then((report) => {
        setState({ status: "success", report });
      })
      .catch((error: unknown) => {
        const code = error instanceof Error ? error.message : "unknown_error";
        setState({ status: "error", code });
      })
      .finally(() => {
        isRunningRef.current = false;
      });
  }, []);

  return (
    <section className="min-w-0" data-testid="mike49-audit-panel">
      <SettingsSectionHeader
        title="MIKE-49 fixture audit (simulated data)"
        description={
          <>
            Runs an offline, fixture-only agent-turn-metric verification demo
            against disposable in-memory keys. No real key, relay, or archive is
            touched — this never runs automatically.
          </>
        }
      />
      <SettingsOptionGroup title="Run">
        <div className="flex flex-col gap-3 p-3">
          <div className="flex items-center gap-2">
            <Button
              data-testid="mike49-run-button"
              disabled={state.status === "loading"}
              onClick={onClick}
              size="sm"
            >
              {state.status === "loading" ? "Running…" : "Run fixture audit"}
            </Button>
            {state.status === "loading" && (
              <span
                className="text-xs text-muted-foreground"
                data-testid="mike49-loading"
              >
                Running simulated fixture audit…
              </span>
            )}
          </div>

          {state.status === "error" && (
            <div
              className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive"
              data-testid="mike49-error"
            >
              {describeErrorCode(state.code)}
            </div>
          )}

          {state.status === "success" && <ReportView report={state.report} />}
        </div>
      </SettingsOptionGroup>
    </section>
  );
}
