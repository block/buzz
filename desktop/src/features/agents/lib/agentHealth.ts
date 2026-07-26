import type {
  ManagedAgent,
  PresenceStatus,
  RespondToMode,
} from "@/shared/api/types";
import type { ProfileChannelLink } from "@/features/profile/ui/UserProfilePanelUtils";

export type AgentHealthAvailability = "available" | "unknown" | "unavailable";

export type AgentHealthField = {
  key: string;
  label: string;
  value: string;
  availability: AgentHealthAvailability;
  detail?: string;
};

export type AgentHealthWarning = {
  key: string;
  label: string;
  severity: "warning" | "error";
};

export type AgentHealthSnapshot = {
  fields: AgentHealthField[];
  warnings: AgentHealthWarning[];
};

function formatRespondTo(mode: RespondToMode): string {
  if (mode === "owner-only") return "Owner only";
  if (mode === "allowlist") return "Selected people";
  return "Anyone";
}

function formatTimestamp(value: string | null): string {
  if (!value) return "No run recorded";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Unknown";
  return date.toLocaleString();
}

export function buildAgentHealthSnapshot({
  agent,
  channels,
  channelsError,
  channelsLoading,
  presenceLoaded,
  presenceStatus,
}: {
  agent: ManagedAgent;
  channels: ProfileChannelLink[];
  channelsError: boolean;
  channelsLoading: boolean;
  presenceLoaded: boolean;
  presenceStatus: PresenceStatus | undefined;
}): AgentHealthSnapshot {
  const channelField: AgentHealthField = channelsError
    ? {
        key: "channels",
        label: "Channel memberships",
        value: "Unavailable",
        availability: "unavailable",
        detail: "Buzz could not load channel memberships.",
      }
    : channelsLoading
      ? {
          key: "channels",
          label: "Channel memberships",
          value: "Loading",
          availability: "unknown",
        }
      : {
          key: "channels",
          label: "Channel memberships",
          value:
            channels.length > 0
              ? channels.map((channel) => `#${channel.name}`).join(", ")
              : "None",
          availability: "available",
        };

  const warnings: AgentHealthWarning[] = [];
  if (agent.personaOrphaned) {
    warnings.push({
      key: "persona-orphaned",
      label: "Linked configuration is missing",
      severity: "error",
    });
  }
  if (agent.needsRestart) {
    warnings.push({
      key: "restart",
      label: "Restart required to apply saved configuration",
      severity: "warning",
    });
  }
  if (agent.personaOutOfDate) {
    warnings.push({
      key: "persona-out-of-date",
      label: "Agent was created from an older configuration",
      severity: "warning",
    });
  }
  if (agent.lastError) {
    warnings.push({
      key: "last-error",
      label: agent.lastError,
      severity: "error",
    });
  }

  return {
    fields: [
      {
        key: "avatar",
        label: "Avatar",
        value: agent.avatarUrl ? "Configured" : "Not configured",
        availability: agent.avatarUrl ? "available" : "unavailable",
      },
      {
        key: "saved-instructions",
        label: "Saved instructions",
        value: agent.systemPrompt?.trim() ? "Configured" : "Not configured",
        availability: "available",
      },
      {
        key: "configuration-version",
        label: "Configuration version",
        value: "Unavailable",
        availability: "unavailable",
        detail: "Managed agents do not currently expose a saved version.",
      },
      {
        key: "configuration-updated",
        label: "Configuration updated",
        value: formatTimestamp(agent.updatedAt),
        availability: "available",
      },
      {
        key: "runtime",
        label: "Runtime",
        value: agent.agentCommand || "Unknown",
        availability: agent.agentCommand ? "available" : "unknown",
      },
      {
        key: "provider",
        label: "Provider",
        value: agent.provider || "Unknown",
        availability: agent.provider ? "available" : "unknown",
      },
      {
        key: "model",
        label: "Model",
        value: agent.model || "Unknown",
        availability: agent.model ? "available" : "unknown",
      },
      {
        key: "response-scope",
        label: "Response access",
        value: formatRespondTo(agent.respondTo),
        availability: "available",
      },
      channelField,
      {
        key: "last-successful-mention",
        label: "Last successful mention",
        value: "Unavailable",
        availability: "unavailable",
        detail:
          "Buzz does not currently persist a per-agent successful-mention timestamp.",
      },
      {
        key: "last-run",
        label: "Last run",
        value: formatTimestamp(agent.lastStartedAt),
        availability: agent.lastStartedAt ? "available" : "unknown",
        detail: agent.lastStartedAt
          ? "Most recent managed process start."
          : "No managed process start has been recorded.",
      },
      {
        key: "presence",
        label: "Presence",
        value: !presenceLoaded
          ? "Loading"
          : presenceStatus
            ? presenceStatus[0]?.toUpperCase() + presenceStatus.slice(1)
            : "Unknown",
        availability:
          presenceLoaded && presenceStatus ? "available" : "unknown",
      },
    ],
    warnings,
  };
}
