import * as React from "react";
import { AlertCircle, ChevronDown, Upload } from "lucide-react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type {
  TeamSnapshotImportPreview,
  TeamSnapshotImportResult,
} from "@/shared/api/tauriTeams";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog, DialogClose } from "@/shared/ui/dialog";
import {
  deriveImportPhase,
  getProfileSyncFailures,
} from "./teamSnapshotImport.lib";
import {
  AgentDefinitionDetails,
  DefinitionMarkdown,
  getAgentInstructionSummary,
} from "./AgentDefinitionDetails";

type TeamSnapshotImportDialogProps = {
  open: boolean;
  /** Preview data loaded by the caller before opening. */
  preview: TeamSnapshotImportPreview;
  /** True while the confirm mutation is in-flight. */
  isConfirming: boolean;
  /** Set when the confirm mutation has returned a result. */
  result: TeamSnapshotImportResult | null;
  /** Error from the confirm mutation, if any. */
  confirmError: string | null;
  /** Called with keepAllowlist when user clicks Import. */
  onConfirm: (keepAllowlist: boolean) => void;
  onOpenChange: (open: boolean) => void;
};

// ── Component ─────────────────────────────────────────────────────────────────

export function TeamSnapshotImportDialog({
  open,
  preview,
  isConfirming,
  result,
  confirmError,
  onConfirm,
  onOpenChange,
}: TeamSnapshotImportDialogProps) {
  const [keepAllowlist, setKeepAllowlist] = React.useState(false);

  // Reset choice whenever the dialog opens with new data.
  React.useEffect(() => {
    if (open) {
      setKeepAllowlist(false);
    }
  }, [open]);

  const phase = deriveImportPhase(result, isConfirming);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        aria-describedby={undefined}
        className="max-w-xl"
        contentClassName="pt-3"
        data-testid="team-snapshot-import-dialog"
        footer={
          <ImportDialogFooter
            isConfirming={isConfirming}
            keepAllowlist={keepAllowlist}
            onConfirm={onConfirm}
            phase={phase}
          />
        }
        footerClassName="border-t-0 pt-0"
        footerTestId="team-snapshot-import-footer"
        headerClassName="pb-2"
        scrollAreaTestId="team-snapshot-import-scroll-area"
        showCloseButton={false}
        style={{ maxHeight: "min(42rem, 85vh)" }}
        title={phase === "result" ? "Team imported" : "Import team snapshot"}
      >
        {phase === "preview" ? (
          <div className="space-y-3">
            <PreviewBody
              preview={preview}
              keepAllowlist={keepAllowlist}
              onKeepAllowlistChange={setKeepAllowlist}
            />
            {confirmError ? (
              <div
                className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
                data-testid="team-snapshot-import-confirm-error"
              >
                <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
                <p>{confirmError}</p>
              </div>
            ) : null}
          </div>
        ) : phase === "confirming" ? (
          <div className="py-4 text-center text-sm text-muted-foreground">
            Creating team…
          </div>
        ) : result !== null ? (
          <ResultBody result={result} />
        ) : null}
      </ChooserDialogContent>
    </Dialog>
  );
}

function ImportDialogFooter({
  phase,
  isConfirming,
  keepAllowlist,
  onConfirm,
}: {
  phase: ReturnType<typeof deriveImportPhase>;
  isConfirming: boolean;
  keepAllowlist: boolean;
  onConfirm: (keepAllowlist: boolean) => void;
}) {
  if (phase === "result") {
    return (
      <div className="flex w-full justify-end">
        <DialogClose asChild>
          <Button type="button">Close</Button>
        </DialogClose>
      </div>
    );
  }

  return (
    <div className="flex w-full items-center justify-end gap-3">
      <DialogClose asChild>
        <Button disabled={isConfirming} type="button" variant="outline">
          Cancel
        </Button>
      </DialogClose>
      <Button
        data-testid="team-snapshot-import-confirm"
        disabled={isConfirming}
        onClick={() => onConfirm(keepAllowlist)}
        type="button"
      >
        <Upload className="h-4 w-4" />
        {phase === "confirming" ? "Importing…" : "Import"}
      </Button>
    </div>
  );
}

// ── Preview body ──────────────────────────────────────────────────────────────

function PreviewBody({
  preview,
  keepAllowlist,
  onKeepAllowlistChange,
}: {
  preview: TeamSnapshotImportPreview;
  keepAllowlist: boolean;
  onKeepAllowlistChange: (v: boolean) => void;
}) {
  return (
    <div className="space-y-5 py-1">
      <div className="space-y-2" data-testid="team-snapshot-import-details">
        <p className="text-base font-semibold tracking-tight">{preview.name}</p>
        {preview.description ? (
          <div data-testid="team-snapshot-import-description">
            <DefinitionMarkdown content={preview.description} />
          </div>
        ) : null}
        {preview.instructions ? (
          <div className="space-y-1 pt-1">
            <p className="text-xs font-medium text-foreground">
              Team instructions
            </p>
            <div data-testid="team-snapshot-import-instructions">
              <DefinitionMarkdown content={preview.instructions} />
            </div>
          </div>
        ) : null}
      </div>

      <p className="text-xs leading-5 text-muted-foreground">
        A new team will be created with fresh keypairs for all members. The
        imported team is independent of the source — identity never travels.
      </p>

      {preview.members.length > 0 ? (
        <div className="space-y-2">
          <p className="text-sm font-medium text-foreground">Members</p>
          <div
            className="overflow-hidden rounded-xl border border-border/70 bg-background/70"
            data-testid="team-snapshot-import-members"
          >
            {preview.members.map((member, idx) => {
              const summary = getAgentInstructionSummary(
                member.summary,
                member.systemPrompt,
              );

              return (
                <details
                  className="group/member relative after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden"
                  data-testid={`team-snapshot-import-member-${idx}`}
                  // biome-ignore lint/suspicious/noArrayIndexKey: snapshots do not include a stable member id and names may duplicate
                  key={idx}
                >
                  <summary className="flex min-h-14 cursor-pointer list-none items-center gap-3 px-4 py-3.5 text-left transition-colors hover:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
                    <ProfileAvatar
                      avatarUrl={member.avatarUrl}
                      className="h-8 w-8 text-xs shadow-none"
                      label={member.displayName}
                      testId={`team-snapshot-import-member-avatar-${idx}`}
                    />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium tracking-tight">
                        {member.displayName}
                      </p>
                      {summary ? (
                        <p
                          className="line-clamp-1 text-xs leading-5 text-muted-foreground"
                          data-testid={`team-snapshot-import-member-summary-${idx}`}
                        >
                          {summary}
                        </p>
                      ) : null}
                    </div>
                    <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-open/member:rotate-180" />
                  </summary>
                  <div
                    className="space-y-6 border-t border-border/60 px-4 py-4"
                    data-testid={`team-snapshot-import-member-details-${idx}`}
                  >
                    <AgentDefinitionDetails
                      isBuiltIn={member.isBuiltIn ?? false}
                      model={member.model ?? null}
                      runtime={member.runtime ?? null}
                      systemPrompt={member.systemPrompt ?? ""}
                    />
                  </div>
                </details>
              );
            })}
          </div>
        </div>
      ) : null}

      {/* Allowlist section */}
      {preview.hasSourceAllowlist ? (
        <div
          className="space-y-2 rounded-md border border-border p-3"
          data-testid="team-snapshot-import-allowlist-section"
        >
          <p className="text-sm font-medium">Respond-to allowlist</p>
          <p className="text-xs text-muted-foreground">
            This snapshot includes source-environment pubkey allowlists for one
            or more members. Those identities are not meaningful on your relay.
          </p>
          <div className="flex flex-col gap-1.5">
            <label className="flex cursor-pointer items-center gap-2">
              <input
                checked={!keepAllowlist}
                data-testid="team-snapshot-import-allowlist-clear"
                name="allowlist-choice"
                onChange={() => onKeepAllowlistChange(false)}
                type="radio"
              />
              <span className="text-sm">
                <strong>Clear</strong> — start with empty allowlists (safer)
              </span>
            </label>
            <label className="flex cursor-pointer items-center gap-2">
              <input
                checked={keepAllowlist}
                data-testid="team-snapshot-import-allowlist-keep"
                name="allowlist-choice"
                onChange={() => onKeepAllowlistChange(true)}
                type="radio"
              />
              <span className="text-sm">
                <strong>Keep</strong> — copy source allowlists to new members
              </span>
            </label>
          </div>
        </div>
      ) : null}
    </div>
  );
}

// ── Result body ───────────────────────────────────────────────────────────────

function ResultBody({ result }: { result: TeamSnapshotImportResult }) {
  const totalMemoryErrors = result.members.reduce(
    (sum, m) => sum + m.memoryErrors.length,
    0,
  );
  const totalMemoryWritten = result.members.reduce(
    (sum, m) => sum + m.memoryWritten,
    0,
  );
  const totalMemoryTotal = result.members.reduce(
    (sum, m) => sum + m.memoryTotal,
    0,
  );
  const hasPartialMemory =
    totalMemoryTotal > 0 && totalMemoryWritten < totalMemoryTotal;
  const profileSyncFailures = getProfileSyncFailures(result.members);

  return (
    <div className="space-y-3 py-1">
      <p className="text-sm">
        <span className="font-medium">{result.team.name}</span> was created
        {profileSyncFailures.length > 0
          ? `, but ${profileSyncFailures.length} member${profileSyncFailures.length === 1 ? "" : "s"} failed to publish ${profileSyncFailures.length === 1 ? "a profile" : "profiles"}.`
          : ` successfully with ${result.members.length} member${result.members.length === 1 ? "" : "s"}.`}
      </p>

      {profileSyncFailures.length > 0 ? (
        <div
          className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
          data-testid="team-snapshot-import-profile-sync-errors"
        >
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="flex flex-col gap-1">
            <p>Profile sync failed for:</p>
            <ul className="mt-1 max-h-32 space-y-0.5 overflow-y-auto text-xs">
              {profileSyncFailures.map((m) => (
                <li key={m.pubkey} className="break-all font-mono">
                  {m.displayName}: {m.profileSyncError}
                </li>
              ))}
            </ul>
          </div>
        </div>
      ) : null}

      {totalMemoryTotal > 0 ? (
        hasPartialMemory ? (
          <div
            className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-400"
            data-testid="team-snapshot-import-partial-memory"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="flex flex-col gap-1">
              <p>
                Memory partially restored: {totalMemoryWritten} of{" "}
                {totalMemoryTotal} entr
                {totalMemoryTotal === 1 ? "y" : "ies"} written across all
                members.
              </p>
              {totalMemoryErrors > 0 ? (
                <ul
                  className="mt-1 max-h-32 space-y-0.5 overflow-y-auto text-xs"
                  data-testid="team-snapshot-import-memory-errors"
                >
                  {result.members.flatMap((member) =>
                    member.memoryErrors.map((err, index) => (
                      <li
                        // biome-ignore lint/suspicious/noArrayIndexKey: error strings may duplicate; pubkey+index is the stable composite key
                        key={`${member.pubkey}:${index}`}
                        className="break-all font-mono"
                      >
                        {member.displayName}: {err}
                      </li>
                    )),
                  )}
                </ul>
              ) : null}
            </div>
          </div>
        ) : (
          <p
            className="text-xs text-muted-foreground"
            data-testid="team-snapshot-import-memory-success"
          >
            {totalMemoryTotal} memory entr
            {totalMemoryTotal === 1 ? "y" : "ies"} restored across all members.
          </p>
        )
      ) : null}
    </div>
  );
}
