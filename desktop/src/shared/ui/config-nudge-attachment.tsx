import { AlertTriangle } from "lucide-react";

import { requestOpenEditAgent } from "@/features/agents/openEditAgentEvent";
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
 * Inline card rendered when the desktop detects a `buzz:config-nudge`
 * sentinel in a kind:9 message body.
 *
 * Uses the `Attachment` primitive's built-in `state="error"` destructive-tint
 * variant so it is visually distinct and consistent with other error states in
 * the system.
 *
 * The card's trigger opens the Edit Agent dialog for the agent by:
 * 1. Calling `openProfilePanel(pubkey)` (from ProfilePanelContext) to ensure
 *    the profile panel is visible.
 * 2. Dispatching `requestOpenEditAgent(pubkey)` so that `UserProfilePanel`
 *    auto-opens the edit dialog once it mounts with that pubkey.
 */
export function ConfigNudgeCard({
  className,
  nudge,
}: {
  className?: string;
  nudge: ConfigNudgePayload;
}) {
  const { openProfilePanel } = useProfilePanel();

  const handleOpen = () => {
    openProfilePanel?.(nudge.agent_pubkey);
    requestOpenEditAgent(nudge.agent_pubkey);
  };

  return (
    <Attachment
      className={cn("max-w-[min(100%,32rem)] shrink-0 shadow-none", className)}
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
            <RequirementRow key={requirementKey(req, i)} requirement={req} />
          ))}
        </div>
      </AttachmentContent>
      <AttachmentActions>
        <span className="text-xs text-muted-foreground opacity-0 transition-opacity group-hover/attachment:opacity-100 group-focus-within/attachment:opacity-100">
          Edit Agent
        </span>
      </AttachmentActions>
      <AttachmentTrigger
        aria-label={`Open Edit Agent for ${nudge.agent_name}`}
        onClick={handleOpen}
      />
    </Attachment>
  );
}

function RequirementRow({
  requirement,
}: {
  requirement: ConfigNudgePayload["requirements"][number];
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
        <div className="text-xs leading-4 text-muted-foreground">
          {requirement.setup_copy}
        </div>
      );
  }
}
