import type { LucideIcon } from "lucide-react";
import {
  Activity,
  Copy,
  Cpu,
  Fingerprint,
  MessageSquare,
  Server,
  Terminal,
  UserRound,
} from "lucide-react";
import type * as React from "react";
import { toast } from "sonner";

import { AgentStatusBadge } from "@/features/agents/ui/AgentStatusBadge";
import { truncatePubkey as truncatePubkeyShort } from "@/features/profile/lib/identity";
import type { ProfileSummaryData } from "@/features/profile/ui/profileSummaryTypes";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";

const RUNTIME_LABELS: Record<string, string> = {
  goose: "Goose",
  "claude-code": "Claude Code",
  "codex-acp": "Codex",
  aider: "Aider",
};

export function runtimeLabel(command: string): string {
  return RUNTIME_LABELS[command] ?? command;
}

export async function copyToClipboard(value: string, label?: string) {
  await navigator.clipboard.writeText(value);
  toast.success(label ? `Copied ${label}` : "Copied to clipboard");
}

export type ProfileField = {
  copyValue?: string;
  /**
   * Plain-text representation. Always required so non-visual surfaces (e.g. tooltips,
   * copy-to-clipboard) keep working. When `displayNode` is set, the row renders that
   * instead of the text — but the text still drives the title/tooltip.
   */
  displayValue: string;
  /**
   * Optional rich rendering for the value cell (e.g. a status badge). When present,
   * replaces the plain text node in the row.
   */
  displayNode?: React.ReactNode;
  icon: LucideIcon;
  label: string;
  testId?: string;
};

export function buildPublicFields({
  isBot,
  profile,
  pubkey,
  relayAgent,
}: {
  isBot: boolean;
  profile: ProfileSummaryData;
  pubkey: string;
  relayAgent: RelayAgent | undefined;
}): ProfileField[] {
  const fields: ProfileField[] = [
    {
      copyValue: pubkey,
      displayValue: truncatePubkeyShort(pubkey),
      icon: Fingerprint,
      label: "Public key",
      testId: "user-profile-copy-pubkey",
    },
  ];

  if (profile?.nip05Handle) {
    fields.push({
      copyValue: profile.nip05Handle,
      displayValue: profile.nip05Handle,
      icon: UserRound,
      label: "NIP-05",
      testId: "user-profile-nip05",
    });
  }

  if (isBot && relayAgent?.agentType) {
    fields.push({
      copyValue: relayAgent.agentType,
      displayValue: runtimeLabel(relayAgent.agentType),
      icon: Cpu,
      label: "Agent type",
      testId: "user-profile-agent-type",
    });
  }

  if (relayAgent?.capabilities.length) {
    fields.push({
      copyValue: relayAgent.capabilities.join(", "),
      displayValue: relayAgent.capabilities.join(", "),
      icon: Server,
      label: "Capabilities",
      testId: "user-profile-capabilities",
    });
  }

  return fields;
}

export function buildOwnerFields({
  managedAgent,
  ownerDisplayName,
  ownerHandle,
  presenceLoaded,
  presenceStatus,
  relayAgent,
}: {
  managedAgent: ManagedAgent | undefined;
  ownerDisplayName: string | null;
  ownerHandle: string | null;
  presenceLoaded: boolean;
  presenceStatus: "online" | "away" | "offline" | undefined;
  relayAgent: RelayAgent | undefined;
}): ProfileField[] {
  const fields: ProfileField[] = [];

  if (ownerDisplayName) {
    fields.push({
      copyValue: ownerHandle ?? undefined,
      displayValue: ownerDisplayName,
      icon: UserRound,
      label: "Owned by",
      testId: "user-profile-owned-by",
    });
  }

  if (managedAgent?.agentCommand) {
    fields.push({
      copyValue: managedAgent.agentCommand,
      displayValue: runtimeLabel(managedAgent.agentCommand),
      icon: Terminal,
      label: "Runtime",
      testId: "user-profile-runtime",
    });
  } else if (relayAgent?.agentType) {
    fields.push({
      copyValue: relayAgent.agentType,
      displayValue: runtimeLabel(relayAgent.agentType),
      icon: Terminal,
      label: "Runtime",
      testId: "user-profile-runtime",
    });
  }

  if (managedAgent) {
    fields.push({
      displayValue: managedAgent.status
        .replace(/_/g, " ")
        .replace(/\b\w/g, (char: string) => char.toUpperCase()),
      displayNode: (
        <AgentStatusBadge
          presenceLoaded={presenceLoaded}
          presenceStatus={presenceStatus}
          status={managedAgent.status}
        />
      ),
      icon: Activity,
      label: "Status",
      testId: "user-profile-agent-status",
    });
  }

  if (managedAgent?.model) {
    fields.push({
      copyValue: managedAgent.model,
      displayValue: managedAgent.model,
      icon: Cpu,
      label: "Model",
      testId: "user-profile-model",
    });
  }

  if (managedAgent?.acpCommand) {
    fields.push({
      copyValue: managedAgent.acpCommand,
      displayValue: managedAgent.acpCommand,
      icon: Terminal,
      label: "ACP command",
      testId: "user-profile-acp",
    });
  }

  if (managedAgent?.mcpCommand) {
    fields.push({
      copyValue: managedAgent.mcpCommand,
      displayValue: managedAgent.mcpCommand,
      icon: Terminal,
      label: "MCP command",
      testId: "user-profile-mcp",
    });
  }

  if (managedAgent?.backend.type === "provider") {
    const backendLabel = managedAgent.backend.id;
    fields.push({
      copyValue: backendLabel,
      displayValue: backendLabel,
      icon: Server,
      label: "Backend",
      testId: "user-profile-backend",
    });
  }

  if (managedAgent) {
    fields.push({
      displayValue: managedAgent.startOnAppLaunch ? "Yes" : "No",
      icon: Server,
      label: "Start on launch",
      testId: "user-profile-start-on-launch",
    });
    fields.push({
      displayValue: managedAgent.respondTo.replace(/-/g, " "),
      icon: MessageSquare,
      label: "Respond to",
      testId: "user-profile-respond-to",
    });
  }

  if (managedAgent?.lastError) {
    fields.push({
      copyValue: managedAgent.lastError,
      displayValue: managedAgent.lastError,
      icon: Activity,
      label: "Last error",
      testId: "user-profile-last-error",
    });
  }

  return fields;
}

export function ProfileFieldGroup({ fields }: { fields: ProfileField[] }) {
  const publicKeyLabel = "Public key";
  const ownedByLabel = "Owned by";
  const statusLabel = "Status";
  const orderedFields = [
    ...fields.filter((field) => field.label === publicKeyLabel),
    ...fields.filter((field) => field.label === ownedByLabel),
    ...fields.filter(
      (field) =>
        field.label !== publicKeyLabel &&
        field.label !== ownedByLabel &&
        field.copyValue,
    ),
    ...fields.filter((field) => field.label === statusLabel),
    ...fields.filter((field) => {
      if (
        field.label === publicKeyLabel ||
        field.label === ownedByLabel ||
        field.label === statusLabel
      ) {
        return false;
      }
      return !field.copyValue;
    }),
  ];

  return (
    <section>
      <div className="overflow-hidden rounded-2xl bg-muted/20">
        {orderedFields.map((field) => (
          <ProfileFieldRow field={field} key={field.testId ?? field.label} />
        ))}
      </div>
    </section>
  );
}

function ProfileFieldRow({ field }: { field: ProfileField }) {
  const Icon = field.icon;
  const isCopyable = Boolean(field.copyValue);

  const content = (
    <>
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1 text-left">
        <span className="block text-xs font-medium text-foreground">
          {field.label}
        </span>
        <span
          className="mt-0.5 block truncate text-sm text-muted-foreground"
          title={field.displayValue}
        >
          {field.displayNode ?? field.displayValue}
        </span>
      </span>
      {isCopyable ? (
        <Copy className="h-4 w-4 shrink-0 text-muted-foreground" />
      ) : null}
    </>
  );

  if (isCopyable && field.copyValue) {
    return (
      <button
        aria-label={`Copy ${field.label}`}
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40"
        data-testid={field.testId}
        onClick={() => void copyToClipboard(field.copyValue ?? "", field.label)}
        title={`Copy ${field.label}`}
        type="button"
      >
        {content}
      </button>
    );
  }

  return (
    <div
      className="flex items-center gap-3 px-4 py-3"
      data-testid={field.testId}
    >
      {content}
    </div>
  );
}
