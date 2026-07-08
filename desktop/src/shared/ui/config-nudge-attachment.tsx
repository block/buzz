import { AlertTriangle } from "lucide-react";

import { requestOpenEditAgent } from "@/features/agents/openEditAgentEvent";
import { useAppShell } from "@/app/AppShellContext";
import type { ConfigNudgePayload } from "@/shared/lib/configNudge";
import { cn } from "@/shared/lib/cn";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import {
  Attachment,
  AttachmentActions,
  AttachmentContent,
  AttachmentMedia,
  AttachmentTitle,
  AttachmentTrigger,
} from "@/shared/ui/attachment";

/**
 * Stable key for a requirement row. The combination of surface + primary
 * value uniquely identifies a requirement within a nudge payload.
 * The fallback position index handles edge cases like two identical rows.
 */
function requirementKey(
  req: ConfigNudgePayload["requirements"][number],
  index: number,
): string {
  switch (req.surface) {
    case "env_key":
      return `env_key:${req.key}:${index}`;
    case "normalized_field":
      return `normalized_field:${req.field}:${index}`;
    case "cli_login":
      return `cli_login:${req.probe_args.join(",")}:${index}`;
  }
}

/**
 * Returns true when every requirement in the nudge is a `cli_login` surface.
 * In that case the card routes to Doctor (install/login can't be fixed in
 * Edit Agent), per design decision (A).
 */
function isAllCliLogin(reqs: ConfigNudgePayload["requirements"]): boolean {
  return reqs.length > 0 && reqs.every((r) => r.surface === "cli_login");
}

/**
 * Per-state human-readable copy for a cli_login requirement.
 * Uses the probe_args[0] as a best-effort harness name.
 */
function cliLoginMessage(
  req: Extract<
    ConfigNudgePayload["requirements"][number],
    { surface: "cli_login" }
  >,
): string {
  const harness = req.probe_args[0] ?? "the CLI tool";
  switch (req.availability) {
    case "not_installed":
      return `${harness} isn't installed`;
    case "cli_missing":
      return `${harness} CLI is missing`;
    case "adapter_missing":
      return `${harness} ACP adapter isn't installed`;
    case "available":
      // Tooling is present but authentication is needed — fall back to
      // the backend-supplied copy which has the exact login command.
      return req.setup_copy;
  }
}

/**
 * Inline card rendered when the desktop detects a `buzz:config-nudge`
 * sentinel in a kind:9 message body.
 *
 * Uses the `Attachment` primitive's built-in `state="error"` destructive-tint
 * variant so it is visually distinct and consistent with other error states in
 * the system.
 *
 * Routing:
 * (A) When ALL requirements are `cli_login`, the card trigger opens
 *     Settings → Doctor (install/login can't be fixed in Edit Agent).
 * (B) Otherwise, the card trigger opens Edit Agent, and each `cli_login` row
 *     renders its own "Open Doctor →" inline CTA that routes to Doctor
 *     regardless of the card-level trigger.
 */
export function ConfigNudgeCard({
  className,
  nudge,
}: {
  className?: string;
  nudge: ConfigNudgePayload;
}) {
  const { openProfilePanel } = useProfilePanel();
  const { onOpenSettings } = useAppShell();

  const allCliLogin = isAllCliLogin(nudge.requirements);

  const handleOpen = () => {
    if (allCliLogin) {
      // (A) Pure cli_login card — route to Doctor.
      onOpenSettings?.("doctor");
    } else {
      openProfilePanel?.(nudge.agent_pubkey);
      requestOpenEditAgent(nudge.agent_pubkey);
    }
  };

  const handleOpenDoctor = (e: React.MouseEvent) => {
    // (B) Inline Doctor CTA — stop propagation so the card trigger doesn't
    // double-fire (which would also open Edit Agent on mixed cards).
    e.stopPropagation();
    onOpenSettings?.("doctor");
  };

  // CTA label shown in AttachmentActions.
  // Always-visible pill (replaces the opacity-0 fade-in hint) so the card
  // reads as clickable at rest (affordance #3).
  const ctaLabel = allCliLogin ? "Open Doctor →" : "Edit Agent →";

  return (
    <Attachment
      className={cn(
        "max-w-[min(100%,32rem)] shrink-0 shadow-none",
        // Affordance #2: cursor-pointer + subtle hover lift.
        "cursor-pointer hover:shadow-sm",
        className,
      )}
      orientation="horizontal"
      state="error"
    >
      <AttachmentMedia className="text-destructive">
        <AlertTriangle aria-hidden="true" className="h-4 w-4" />
      </AttachmentMedia>
      <AttachmentContent>
        <AttachmentTitle className="whitespace-normal text-destructive line-clamp-2">
          {nudge.agent_name} needs configuration
        </AttachmentTitle>
        <div className="mt-1 flex flex-col gap-0.5">
          {nudge.requirements.map((req, i) => (
            <RequirementRow
              key={requirementKey(req, i)}
              onOpenDoctor={handleOpenDoctor}
              requirement={req}
              showRowDoctorCta={!allCliLogin}
            />
          ))}
        </div>
      </AttachmentContent>
      <AttachmentActions>
        {/* Affordance #3: always-visible CTA text (was opacity-0 fade-in). */}
        <span className="text-xs text-muted-foreground">{ctaLabel}</span>
      </AttachmentActions>
      <AttachmentTrigger
        aria-label={
          allCliLogin
            ? `Open Doctor settings for ${nudge.agent_name}`
            : `Open Edit Agent for ${nudge.agent_name}`
        }
        onClick={handleOpen}
      />
    </Attachment>
  );
}

function RequirementRow({
  onOpenDoctor,
  requirement,
  showRowDoctorCta,
}: {
  onOpenDoctor: (e: React.MouseEvent) => void;
  requirement: ConfigNudgePayload["requirements"][number];
  showRowDoctorCta: boolean;
}) {
  switch (requirement.surface) {
    case "env_key":
      return (
        <div className="text-xs leading-4 text-muted-foreground [overflow-wrap:anywhere]">
          Set{" "}
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">
            {requirement.key}
          </code>{" "}
          in Edit Agent → Environment variables
        </div>
      );
    case "normalized_field":
      return (
        <div className="text-xs leading-4 text-muted-foreground [overflow-wrap:anywhere]">
          Set the <strong>{requirement.field}</strong> field in Edit Agent
          dropdowns
        </div>
      );
    case "cli_login":
      return (
        <div className="flex items-center gap-2 text-xs leading-4 text-muted-foreground">
          <span className="flex-1 [overflow-wrap:anywhere]">
            {cliLoginMessage(requirement)}
          </span>
          {/* (B) Inline Doctor CTA — shown only on mixed cards where the
              card-level trigger opens Edit Agent. When allCliLogin is true the
              card trigger already routes to Doctor; the per-row button is
              redundant and is suppressed. stopPropagation prevents double-fire
              on mixed cards where both card and row CTAs are visible. */}
          {showRowDoctorCta && (
            <button
              className="relative z-20 shrink-0 font-medium text-destructive hover:underline"
              onClick={onOpenDoctor}
              type="button"
            >
              Open Doctor →
            </button>
          )}
        </div>
      );
  }
}
