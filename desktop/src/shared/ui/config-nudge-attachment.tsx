import { AlertTriangle } from "lucide-react";

import {
  requestOpenEditAgent,
  type EditAgentFocusTarget,
} from "@/features/agents/openEditAgentEvent";
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
    case "cli_config_invalid":
      return `cli_config_invalid:${req.probe_args.join(",")}:${index}`;
    case "git_bash":
      return `git_bash:${index}`;
  }
}

/**
 * Returns true when every requirement in the nudge is a `cli_login` surface.
 * Non-authOnly all-cli_login cards (at least one install-state row) route to
 * Doctor — install/login problems can't be fixed in Edit Agent. AuthOnly cards
 * (every row is `availability === "available"`) are purely informational and
 * do not route anywhere.
 */
function hasGitBashRequirement(
  reqs: ConfigNudgePayload["requirements"],
): boolean {
  return reqs.some((r) => r.surface === "git_bash");
}

function isAllCliLogin(reqs: ConfigNudgePayload["requirements"]): boolean {
  return reqs.length > 0 && reqs.every((r) => r.surface === "cli_login");
}

export function shouldOpenDoctor(
  reqs: ConfigNudgePayload["requirements"],
): boolean {
  return isAllCliLogin(reqs) || hasGitBashRequirement(reqs);
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
 * Returns true when every requirement is a `cli_config_invalid` surface.
 * Config-invalid cards are purely informational — the user must edit an
 * external file; there is no in-app destination that can fix it.
 */
function isAllConfigInvalid(reqs: ConfigNudgePayload["requirements"]): boolean {
  return (
    reqs.length > 0 && reqs.every((r) => r.surface === "cli_config_invalid")
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
 * Derive a field-focus target from the first actionable requirement.
 * `cli_login` requirements don't map to a focusable Edit Agent field,
 * so they are skipped. Returns `undefined` for cli_login-only nudges.
 */
function firstFocusTarget(
  requirements: ConfigNudgePayload["requirements"],
): EditAgentFocusTarget | undefined {
  for (const req of requirements) {
    if (req.surface === "env_key") {
      return { type: "env_key", key: req.key };
    }
    if (req.surface === "normalized_field") {
      return { type: "normalized_field", field: req.field };
    }
  }
  return undefined;
}

/**
 * Derive a field-focus target from a SINGLE requirement.
 * Mirrors `firstFocusTarget` but operates on one row — used so per-row
 * Edit Agent CTAs focus the field that row describes, not the first editable
 * field on the card.
 * Returns `undefined` for `cli_login` requirements (Doctor, not Edit Agent).
 */
export function focusTargetForRequirement(
  req: ConfigNudgePayload["requirements"][number],
): EditAgentFocusTarget | undefined {
  if (req.surface === "env_key") {
    return { type: "env_key", key: req.key };
  }
  if (req.surface === "normalized_field") {
    return { type: "normalized_field", field: req.field };
  }
  return undefined;
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
 * (A) Any card with a `git_bash` requirement, or one whose requirements are all
 *     install-state `cli_login`, opens Settings → Doctor. A card-level
 *     "Open Doctor →" label in `AttachmentActions` confirms the action at rest.
 * (A-auth) A card whose requirements are all available `cli_login` surfaces is
 *     purely informational: Doctor cannot authenticate a CLI, and `setup_copy`
 *     already gives the needed command.
 * (B) Other mixed cards open Edit Agent as the card-level fallback. Their rows
 *     carry inline CTAs for the matching destination: install-state `cli_login`
 *     opens Doctor; `env_key` and `normalized_field` open Edit Agent. A
 *     `git_bash` row is covered by the card-level Doctor route, so it does not
 *     render a redundant row action.
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
  const opensDoctor = shouldOpenDoctor(nudge.requirements);
  const authOnly = isAuthOnly(nudge.requirements);
  const allConfigInvalid = isAllConfigInvalid(nudge.requirements);
  // Any card that is purely informational (auth-only or all-config-invalid)
  // has no clickable destination — treat them the same for affordance/routing.
  const informationalOnly = authOnly || allConfigInvalid;

  const openDoctor = () => {
    if (!onOpenSettings) {
      console.warn(
        "[ConfigNudgeCard] onOpenSettings is null — Doctor deep-link unavailable on this surface",
      );
    }
    onOpenSettings?.("doctor");
  };

  const openEditAgent = (focus?: EditAgentFocusTarget) => {
    openProfilePanel?.(nudge.agent_pubkey);
    requestOpenEditAgent(nudge.agent_pubkey, focus);
  };

  const handleOpen = () => {
    if (shouldOpenDoctor(nudge.requirements)) {
      // Git Bash and install-state CLI requirements both resolve in Doctor.
      // Informational-only cards never mount this trigger.
      openDoctor();
    } else {
      // (B) Mixed card — card-level fallback: focus the first editable field.
      openEditAgent(firstFocusTarget(nudge.requirements));
    }
  };

  const handleOpenDoctor = (e: React.MouseEvent) => {
    // (B) Per-row Doctor CTA — stop propagation so the card trigger doesn't
    // double-fire to Edit Agent on mixed cards.
    e.stopPropagation();
    openDoctor();
  };

  const handleOpenEditAgent = (
    e: React.MouseEvent,
    focus: EditAgentFocusTarget | undefined,
  ) => {
    // (B) Per-row Edit Agent CTA — focus the field this specific row describes.
    // Stop propagation so the card trigger doesn't double-fire.
    e.stopPropagation();
    openEditAgent(focus);
  };

  return (
    <Attachment
      className={cn(
        "max-w-[min(100%,32rem)] shrink-0 shadow-none",
        // Affordance: cursor-pointer + subtle hover lift — omitted for
        // informational-only cards which have no click destination.
        !informationalOnly && "cursor-pointer hover:shadow-sm",
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
      {/* (A) Doctor-routed cards have one card-level CTA. Informational-only
          cards have none; other mixed cards render their own row CTAs. */}
      {opensDoctor && !informationalOnly && (
        <AttachmentActions className="items-end self-end">
          <span className="text-xs text-muted-foreground">Open Doctor →</span>
        </AttachmentActions>
      )}
      {/* Informational-only cards are purely informational — no trigger, no routing. */}
      {!informationalOnly && (
        <AttachmentTrigger
          aria-label={
            opensDoctor
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
  onOpenEditAgent: (
    e: React.MouseEvent,
    focus: EditAgentFocusTarget | undefined,
  ) => void;
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
              onClick={(e) =>
                onOpenEditAgent(e, focusTargetForRequirement(requirement))
              }
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
              onClick={(e) =>
                onOpenEditAgent(e, focusTargetForRequirement(requirement))
              }
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
    case "git_bash":
      return (
        <div className="flex items-center gap-2 text-xs leading-4 text-muted-foreground">
          <span className="flex-1 [overflow-wrap:anywhere]">
            Git for Windows is required for buzz-agent shell tools
          </span>
        </div>
      );
    case "cli_config_invalid": {
      // Config-invalid rows are purely informational — the user must edit an
      // external file. No Doctor CTA (Doctor can't repair ~/.codex/config.toml)
      // and no Edit Agent CTA (the field isn't managed by Buzz).
      const cli = requirement.probe_args[0] ?? "the CLI";
      const configFile = `~/.${cli}/config.toml`;
      return (
        <div className="flex items-center gap-2 text-xs leading-4 text-muted-foreground">
          <span className="flex-1 [overflow-wrap:anywhere]">
            {configFile} is invalid:{" "}
            <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">
              {requirement.diagnostic}
            </code>{" "}
            — fix the config and restart the agent
          </span>
        </div>
      );
    }
  }
}
