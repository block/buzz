import * as React from "react";
import { stringify as yamlStringify } from "yaml";

import {
  useCreateWorkflowMutation,
  useUpdateWorkflowMutation,
} from "@/features/workflows/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, Workflow } from "@/shared/api/types";
import { getRelayHttpUrl } from "@/shared/api/tauri";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { ChannelCombobox } from "./ChannelCombobox";
import { WorkflowFormBuilder } from "./WorkflowFormBuilder";
import { WorkflowWebhookSecretDialog } from "./WorkflowWebhookSecretDialog";
import {
  canAuthorWebhookWorkflow,
  getChannelRoleForPubkey,
  workflowUsesCallWebhook,
} from "./workflowAuthz";
import { FieldLabel } from "./workflowFormPrimitives";

type DialogMode = "create" | "edit" | "duplicate";

type WorkflowDialogProps = {
  channels: Channel[];
  mode: DialogMode;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  workflow?: Workflow | null;
};

function getInitialYaml(
  mode: DialogMode,
  workflow: Workflow | null | undefined,
): string {
  if (!workflow) return "";
  const def = { ...workflow.definition };
  if (mode === "duplicate") {
    def.name = `${def.name ?? workflow.name} (copy)`;
  }
  return yamlStringify(def);
}

const TITLES: Record<DialogMode, string> = {
  create: "Create Workflow",
  edit: "Edit Workflow",
  duplicate: "Duplicate Workflow",
};

const SUBMIT_LABELS: Record<DialogMode, string> = {
  create: "Create",
  edit: "Save",
  duplicate: "Create Copy",
};

const PENDING_LABELS: Record<DialogMode, string> = {
  create: "Creating...",
  edit: "Saving...",
  duplicate: "Creating...",
};

export function WorkflowDialog({
  channels,
  mode,
  onOpenChange,
  open,
  workflow,
}: WorkflowDialogProps) {
  const channelId =
    mode === "edit" && workflow?.channelId
      ? workflow.channelId
      : (channels[0]?.id ?? "");

  const [selectedChannelId, setSelectedChannelId] = React.useState(channelId);
  const [yamlDefinition, setYamlDefinition] = React.useState(() =>
    getInitialYaml(mode, workflow),
  );
  const [savedWebhookInfo, setSavedWebhookInfo] = React.useState<{
    relayHttpUrl: string;
    webhookSecret: string;
    workflowId: string;
  } | null>(null);

  const createMutation = useCreateWorkflowMutation(selectedChannelId);
  const updateMutation = useUpdateWorkflowMutation(
    workflow?.id ?? "",
    workflow?.revision ?? "",
  );
  const mutation = mode === "edit" ? updateMutation : createMutation;

  const selectedChannel =
    channels.find((c) => c.id === selectedChannelId) ?? null;
  const identityQuery = useIdentityQuery();
  const usesWebhookAction = React.useMemo(
    () => workflowUsesCallWebhook(yamlDefinition),
    [yamlDefinition],
  );
  const membersQuery = useChannelMembersQuery(
    selectedChannelId || null,
    open && usesWebhookAction,
  );
  const selectedChannelRole = getChannelRoleForPubkey(
    membersQuery.data,
    identityQuery.data?.pubkey,
  );
  const webhookAuthzBlocked =
    usesWebhookAction &&
    membersQuery.isSuccess &&
    !canAuthorWebhookWorkflow(selectedChannelRole);

  const defaultChannelId = channels[0]?.id ?? "";
  const workflowChannelId = workflow?.channelId ?? null;
  const resetCreate = createMutation.reset;
  const resetUpdate = updateMutation.reset;

  // Re-initialize when dialog opens or workflow/mode changes
  React.useEffect(() => {
    if (open) {
      const newChannelId =
        mode === "edit" && workflowChannelId
          ? workflowChannelId
          : defaultChannelId;
      setSelectedChannelId(newChannelId);
      setYamlDefinition(getInitialYaml(mode, workflow));
      setSavedWebhookInfo(null);
      resetCreate();
      resetUpdate();
    }
  }, [
    open,
    mode,
    workflow,
    workflowChannelId,
    defaultChannelId,
    resetCreate,
    resetUpdate,
  ]);

  const handleOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      if (!nextOpen) {
        resetCreate();
        resetUpdate();
      }
      onOpenChange(nextOpen);
    },
    [onOpenChange, resetCreate, resetUpdate],
  );

  async function handleSubmit() {
    if (!selectedChannelId || !yamlDefinition.trim() || webhookAuthzBlocked) {
      return;
    }

    try {
      const saved = await mutation.mutateAsync(yamlDefinition);
      handleOpenChange(false);
      if (saved.webhookSecret) {
        const relayHttpUrl = await getRelayHttpUrl();
        setSavedWebhookInfo({
          relayHttpUrl,
          webhookSecret: saved.webhookSecret,
          workflowId: saved.workflow.id,
        });
      }
    } catch {
      // React Query stores the error; keep the dialog open.
    }
  }

  const showChannelSelector = mode !== "edit" && channels.length > 1;
  const showChannelInfo = mode !== "edit" && channels.length === 1;

  return (
    <>
      <Dialog onOpenChange={handleOpenChange} open={open}>
        <DialogContent className="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-lg">
          <DialogHeader className="flex-shrink-0">
            <DialogTitle>{TITLES[mode]}</DialogTitle>
            <DialogDescription>
              {mode === "edit"
                ? "Modify the workflow definition."
                : channels.length === 1
                  ? "Create a workflow scoped to this channel."
                  : "Define a workflow and assign it to a channel."}
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
            {showChannelSelector ? (
              <div className="space-y-1.5">
                <FieldLabel htmlFor="wf-channel-select">Channel</FieldLabel>
                <ChannelCombobox
                  channels={channels}
                  disabled={mutation.isPending}
                  id="wf-channel-select"
                  onChange={(value) => {
                    mutation.reset();
                    setSelectedChannelId(value);
                  }}
                  value={selectedChannelId}
                />
                <p className="text-xs text-muted-foreground">
                  {selectedChannel
                    ? `New workflows will belong to ${selectedChannel.name}.`
                    : "Join or create a channel before adding a workflow."}
                </p>
              </div>
            ) : (showChannelInfo || mode === "edit") && selectedChannel ? (
              <p className="text-sm text-muted-foreground">
                {mode === "edit"
                  ? "Editing workflow in"
                  : "This workflow will be created in"}{" "}
                <span className="font-medium text-foreground">
                  {selectedChannel.name}
                </span>
                .
              </p>
            ) : null}

            <WorkflowFormBuilder
              disabled={mutation.isPending}
              onChange={(yaml) => {
                mutation.reset();
                setYamlDefinition(yaml);
              }}
              yaml={yamlDefinition}
            />

            {usesWebhookAction && membersQuery.isError ? (
              <p className="rounded-xl border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-300">
                Could not check your channel role before saving this webhook
                workflow. If the relay rejects it, the exact relay error will
                appear below.
              </p>
            ) : null}
          </div>

          <div className="flex flex-shrink-0 flex-col gap-3 border-t border-border pt-4">
            {webhookAuthzBlocked ? (
              <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                This workflow uses <code>call_webhook</code>, which the relay
                only allows channel owners or admins to save. Your current role
                in {selectedChannel?.name ?? "this channel"} is{" "}
                <code>{selectedChannelRole ?? "unknown"}</code>. Switch to the
                owner identity or ask an owner/admin to save it.
              </p>
            ) : null}

            {mutation.error instanceof Error ? (
              <p
                aria-live="polite"
                className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
              >
                Save failed: {mutation.error.message}
              </p>
            ) : null}

            <div className="flex justify-end gap-2">
              <Button
                onClick={() => handleOpenChange(false)}
                type="button"
                variant="outline"
              >
                Cancel
              </Button>
              <Button
                disabled={
                  !selectedChannelId ||
                  !yamlDefinition.trim() ||
                  mutation.isPending ||
                  webhookAuthzBlocked
                }
                onClick={handleSubmit}
                type="button"
              >
                {mutation.isPending
                  ? PENDING_LABELS[mode]
                  : SUBMIT_LABELS[mode]}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {savedWebhookInfo ? (
        <WorkflowWebhookSecretDialog
          onOpenChange={(nextOpen) => {
            if (!nextOpen) {
              setSavedWebhookInfo(null);
            }
          }}
          open
          relayHttpUrl={savedWebhookInfo.relayHttpUrl}
          webhookSecret={savedWebhookInfo.webhookSecret}
          workflowId={savedWebhookInfo.workflowId}
        />
      ) : null}
    </>
  );
}
