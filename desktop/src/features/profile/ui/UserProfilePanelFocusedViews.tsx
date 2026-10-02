import { ArrowUpRight, CircleAlert, UserPlus } from "lucide-react";

import { MemorySection } from "@/features/agent-memory/ui/MemorySection";
import { ManagedAgentLogPanel } from "@/features/agents/ui/ManagedAgentLogPanel";
import {
  type ProfileField,
  ProfileFieldGroup,
  ProfileSectionGroup,
} from "@/features/profile/ui/UserProfilePanelFields";
import { ProfileIngressRow } from "@/features/profile/ui/UserProfilePanelTabs";
import type { ProfileChannelLink } from "@/features/profile/ui/UserProfilePanelUtils";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";

export function MemoryFocusedView({
  agentPubkey,
  variant = "focused",
  viewerIsOwner,
}: {
  agentPubkey: string;
  variant?: "embedded" | "focused";
  viewerIsOwner: boolean | undefined;
}) {
  if (viewerIsOwner !== true) {
    return null;
  }

  return (
    <div className={variant === "focused" ? "pt-4" : undefined}>
      <ProfileSectionGroup testId="user-profile-memories-section">
        <MemorySection
          agentPubkey={agentPubkey}
          variant="grouped"
          viewerIsOwner={viewerIsOwner}
        />
      </ProfileSectionGroup>
    </div>
  );
}

export function ChannelsFocusedView({
  canAddToChannel,
  channels,
  isActionPending,
  isLoading,
  onAddToChannel,
  onOpenChannel,
  variant = "focused",
}: {
  canAddToChannel: boolean;
  channels: ProfileChannelLink[];
  isActionPending: boolean;
  isLoading: boolean;
  onAddToChannel: () => void;
  onOpenChannel: (channelId: string) => void;
  variant?: "embedded" | "focused";
}) {
  return (
    <div className={variant === "focused" ? "pt-4" : undefined}>
      <ProfileSectionGroup testId="user-profile-channels-section">
        {canAddToChannel ? (
          <ProfileIngressRow
            disabled={isActionPending}
            grouped
            icon={UserPlus}
            label="Add to channel"
            onClick={onAddToChannel}
            testId="user-profile-agent-add-channel"
            trailing={isActionPending ? "Working…" : undefined}
          />
        ) : null}
        {isLoading ? (
          <p className="px-4 py-3 text-sm leading-6 text-muted-foreground">
            Loading channels…
          </p>
        ) : channels.length === 0 ? (
          <div
            className={cn(
              "flex flex-col items-center justify-center px-6 text-center",
              canAddToChannel ? "min-h-20 py-4" : "min-h-56 py-10",
            )}
            data-testid="user-profile-channels-empty"
          >
            <UserPlus className="mx-auto h-4 w-4 text-muted-foreground" />
            <p className="mt-3 text-sm font-medium">
              {canAddToChannel
                ? "Add this agent to a channel"
                : "Channels appear here"}
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              {canAddToChannel
                ? "Choose a channel above so it can join the conversation."
                : "Visible memberships appear as this agent joins channels."}
            </p>
          </div>
        ) : (
          <ul
            className="divide-y divide-border/55"
            data-testid="user-profile-channels-list"
          >
            {channels.map((channel) => (
              <li key={channel.id}>
                <button
                  aria-label={`Open #${channel.name}`}
                  className="group flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left text-sm font-medium text-foreground transition-colors hover:bg-muted/40"
                  data-testid={`user-profile-channel-link-${channel.name}`}
                  onClick={() => onOpenChannel(channel.id)}
                  type="button"
                >
                  <span className="min-w-0 flex-1 truncate">
                    #{channel.name}
                  </span>
                  <ArrowUpRight
                    aria-hidden="true"
                    className="h-4 w-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground"
                  />
                </button>
              </li>
            ))}
          </ul>
        )}
      </ProfileSectionGroup>
    </div>
  );
}

export function AgentInfoFocusedView({
  metadataFields,
}: {
  metadataFields: ProfileField[];
}) {
  if (metadataFields.length === 0) {
    return null;
  }

  return (
    <div className="pt-4">
      <ProfileFieldGroup fields={metadataFields} title="Info" />
    </div>
  );
}

export function DiagnosticsFocusedView({
  canOpenAgentLogs,
  fields,
  logContent,
  logError,
  logLoading,
  managedAgent,
}: {
  canOpenAgentLogs: boolean;
  fields: ProfileField[];
  logContent: string | null;
  logError: Error | null;
  logLoading: boolean;
  managedAgent: ManagedAgent | undefined;
}) {
  const hasLog = canOpenAgentLogs && managedAgent !== undefined;
  const lastErrorField = fields.find((field) => field.label === "Last error");
  const detailFields = fields.filter(
    (field) => field.label !== "Last error" && field.label !== "Status",
  );

  if (!lastErrorField && detailFields.length === 0 && !hasLog) {
    return null;
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 pt-4">
      {lastErrorField ? (
        <Alert
          className="flex gap-3"
          data-testid={lastErrorField.testId}
          variant="destructive"
        >
          <CircleAlert className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
          <div className="min-w-0">
            <AlertTitle>Last error</AlertTitle>
            <AlertDescription className="wrap-break-word">
              {lastErrorField.displayValue}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}
      {detailFields.length > 0 ? (
        <ProfileFieldGroup fields={detailFields} />
      ) : null}
      {hasLog ? (
        <div className="min-h-0 flex-1">
          <ManagedAgentLogPanel
            chrome="bare"
            error={logError}
            isLoading={logLoading}
            logContent={logContent}
            selectedAgent={managedAgent}
            variant="inline"
          />
        </div>
      ) : null}
    </div>
  );
}
