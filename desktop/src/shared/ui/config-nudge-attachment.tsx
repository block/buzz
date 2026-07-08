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
 * Returns true when the card is all-cli_login AND every requirement is in the
 * `available` state (tooling installed, just needs login). In this case Doctor
 * has no auth functionality and is a misleading dead-end — the card becomes
 * purely informational (no trigger, no CTA, no pointer/hover affordance).
 */
function isAuthOnly(reqs: ConfigNudgePayload["requirements"]): boolean {
  return (
    reqs.length > 0 &&
    reqs.every(
      (r) => r.surface === "cli_login" && r.availability === "available",
    )
  );
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
 * (A) When ALL requirements are `cli_login` in an install state
 *     (`not_installed` / `cli_missing` / `adapter_missing`): the card trigger
 *     opens Settings → Doctor. A card-level "Open Doctor →" label in
 *     `AttachmentActions` confirms the action at rest.
 * (A-auth) When ALL requirements are `cli_login` with `availability ===
 *     "available"` (tooling installed, just needs login): Doctor has no auth
 *     functionality and would be a misleading dead-end. The card is purely
 *     informational — no trigger, no CTA, no pointer/hover affordance. The
 *     inline copy (`setup_copy`) already tells the user the exact command.
 * (B) Otherwise (mixed card), the card trigger opens Edit Agent as the
 *     card-level fallback. Each row carries its own inline CTA sharing one
 *     right edge so the actions are clearly paired with their requirement:
 *     - `cli_login` rows in an install state → "Open Doctor →" (Doctor).
 *     - `cli_login` rows with `availability === "available"` → no per-row CTA.
 *     - `env_key` / `normalized_field` rows → "Edit Agent →" (Edit Agent).
 *     The `AttachmentActions` column is omitted on mixed cards — per-row CTAs
 *     replace it.
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
  const authOnly = isAuthOnly(nudge.requirements);

  const openDoctor = () => {
    if (!onOpenSettings) {
      console.warn(
        "[ConfigNudgeCard] onOpenSettings is null — Doctor deep-link unavailable on this surface",
      );
    }
    onOpenSettings?.("doctor");
  };

  const openEditAgent = () => {
    openProfilePanel?.(nudge.agent_pubkey);
    requestOpenEditAgent(nudge.agent_pubkey);
  };

  const handleOpen = () => {
    if (allCliLogin) {
      // (A) Pure cli_login card — route to Doctor.
      openDoctor();
    } else {
      // (B) Mixed card — card-level fallback to Edit Agent.
      openEditAgent();
    }
  };

  const handleOpenDoctor = (e: React.MouseEvent) => {
    // (B) Per-row Doctor CTA — stop propagation so the card trigger doesn't
    // double-fire to Edit Agent on mixed cards.
    e.stopPropagation();
    openDoctor();
  };

  const handleOpenEditAgent = (e: React.MouseEvent) => {
    // (B) Per-row Edit Agent CTA — stop propagation so the card trigger
    // doesn't double-fire.
    e.stopPropagation();
    openEditAgent();
  };

  return (
    <Attachment
      className={cn(
        "max-w-[min(100%,32rem)] shrink-0 shadow-none",
        // Affordance: cursor-pointer + subtle hover lift — omitted for auth-only
        // cards which are purely informational (no click destination).
        !authOnly && "cursor-pointer hover:shadow-sm",
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
              allCliLogin={allCliLogin}
              onOpenDoctor={handleOpenDoctor}
              onOpenEditAgent={handleOpenEditAgent}
              requirement={req}
            />
          ))}
        </div>
      </AttachmentContent>
      {/* (A) Install-state all-cli_login card only — single card-level CTA
          confirming the action at rest. Auth-only cards are informational
          (no CTA); mixed cards render per-row CTAs instead. */}
      {allCliLogin && !authOnly && (
        <AttachmentActions className="items-end self-end">
          <span className="text-xs text-muted-foreground">Open Doctor →</span>
        </AttachmentActions>
      )}
      {/* Auth-only cards are purely informational — no trigger, no routing. */}
      {!authOnly && (
        <AttachmentTrigger
          aria-label={
            allCliLogin
              ? `Open Doctor settings for ${nudge.agent_name}`
              : `Open Edit Agent for ${nudge.agent_name}`
          }
          onClick={handleOpen}
        />
      )}
    </Attachment>
  );
}

function RequirementRow({
  allCliLogin,
  onOpenDoctor,
  onOpenEditAgent,
  requirement,
}: {
  allCliLogin: boolean;
  onOpenDoctor: (e: React.MouseEvent) => void;
  onOpenEditAgent: (e: React.MouseEvent) => void;
  requirement: ConfigNudgePayload["requirements"][number];
}) {
  switch (requirement.surface) {
    case "env_key":
      return (
        <div className="flex items-center gap-2 text-xs leading-4 text-muted-foreground">
          <span className="flex-1 [overflow-wrap:anywhere]">
            Set{" "}
            <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">
              {requirement.key}
            </code>{" "}
            in Edit Agent → Environment variables
          </span>
          {!allCliLogin && (
            <button
              className="relative z-20 shrink-0 font-medium text-muted-foreground hover:underline"
              onClick={onOpenEditAgent}
              type="button"
            >
              Edit Agent →
            </button>
          )}
        </div>
      );
    case "normalized_field":
      return (
        <div className="flex items-center gap-2 text-xs leading-4 text-muted-foreground">
          <span className="flex-1 [overflow-wrap:anywhere]">
            Set the <strong>{requirement.field}</strong> field in Edit Agent
            dropdowns
          </span>
          {!allCliLogin && (
            <button
              className="relative z-20 shrink-0 font-medium text-muted-foreground hover:underline"
              onClick={onOpenEditAgent}
              type="button"
            >
              Edit Agent →
            </button>
          )}
        </div>
      );
    case "cli_login":
      return (
        <div className="flex items-center gap-2 text-xs leading-4 text-muted-foreground">
          <span className="flex-1 [overflow-wrap:anywhere]">
            {cliLoginMessage(requirement)}
          </span>
          {/* (B) Per-row Doctor CTA — shown only on mixed cards where the
              card-level trigger opens Edit Agent (not auth-only cards). When
              allCliLogin is true the card trigger already routes to Doctor; the
              per-row button is redundant and is suppressed. Also suppressed for
              `available` cli_login rows — Doctor has no auth functionality and
              the setup_copy already provides the exact login command.
              stopPropagation prevents double-fire on mixed cards where both
              card and row CTAs are visible. */}
          {!allCliLogin && requirement.availability !== "available" && (
            <button
              className="relative z-20 shrink-0 font-medium text-muted-foreground hover:underline"
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
