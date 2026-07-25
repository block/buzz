import * as React from "react";

import {
  consumePendingOpenCreateAgent,
  subscribeOpenCreateAgent,
  type OpenCreateAgentOptions,
} from "@/features/agents/openCreateAgentEvent";
import {
  clearAgentInstallPrefill,
  useAgentInstallPrefill,
} from "@/features/agents/agentInstallPrefill";
import { useChannelsQuery } from "@/features/channels/hooks";
import type { CreatePersonaInput } from "@/shared/api/types";
import { AgentDialog } from "./AgentDialog";
import { createPersonaDialogState } from "./personaDialogState";
import { SecretRevealDialog } from "./SecretRevealDialog";
import { usePersonaActions } from "./usePersonaActions";

/** App-level create flow so contextual entry points do not navigate away. */
export function RequestedAgentCreateDialogs() {
  const personas = usePersonaActions();
  const [targetChannel, setTargetChannel] = React.useState<{
    id: string;
    name: string;
  } | null>(null);
  const [initialValues, setInitialValues] =
    React.useState<CreatePersonaInput | null>(null);
  const [installRequestId, setInstallRequestId] = React.useState<string | null>(
    null,
  );
  const [isOpen, setIsOpen] = React.useState(false);

  const openCreate = React.useEffectEvent((options: OpenCreateAgentOptions) => {
    personas.prepareCreate();
    setInitialValues(null);
    setInstallRequestId(null);
    setTargetChannel(
      options.channelId && options.channelName
        ? { id: options.channelId, name: options.channelName }
        : null,
    );
    setIsOpen(true);
  });

  React.useEffect(() => {
    const pending = consumePendingOpenCreateAgent();
    if (pending) openCreate(pending);
    return subscribeOpenCreateAgent(openCreate);
  }, []);

  // `buzz://install-agent?…` prefill: open the create form seeded with the
  // agent's identity (display name / system prompt) so an external service can
  // offer a one-click install. This ONLY prefills — the owner still reviews and
  // saves the form here; nothing auto-admits the agent.
  const installPrefill = useAgentInstallPrefill();
  const channels = useChannelsQuery({ enabled: installPrefill != null });
  const openInstall = React.useEffectEvent(() => {
    if (!installPrefill) return;
    personas.prepareCreate();
    const base = createPersonaDialogState().initialValues;
    setInitialValues({
      ...base,
      displayName: installPrefill.name ?? base.displayName,
      systemPrompt: installPrefill.systemPrompt ?? base.systemPrompt,
    });
    const channel = installPrefill.channel
      ? (channels.data?.find((c) => c.id === installPrefill.channel) ?? {
          id: installPrefill.channel,
          name: installPrefill.channel,
        })
      : null;
    setTargetChannel(channel ? { id: channel.id, name: channel.name } : null);
    setInstallRequestId(installPrefill.requestId);
    setIsOpen(true);
  });

  React.useEffect(() => {
    if (installPrefill) openInstall();
    // `channels` is read via the useEffectEvent above (latest cache), not a
    // reactive dep: re-firing on channel load would reset an in-progress form.
    // The channels query shares its cache with the sidebar, so the name is
    // normally already present; a not-yet-loaded channel falls back to its id.
  }, [installPrefill]);

  const handleClose = () => {
    setIsOpen(false);
    setTargetChannel(null);
    setInitialValues(null);
    if (installRequestId) {
      clearAgentInstallPrefill(installRequestId);
      setInstallRequestId(null);
    }
  };

  return (
    <>
      {isOpen ? (
        <AgentDialog
          definitionError={
            personas.createPersonaMutation.error instanceof Error
              ? personas.createPersonaMutation.error
              : null
          }
          initialValues={initialValues}
          isDefinitionPending={personas.isPending}
          mode="definition"
          onOpenChange={(open) => {
            if (!open) handleClose();
          }}
          onSubmitDefinition={(input, intent, backendIntent) =>
            personas.handleSubmit(input, intent, backendIntent, targetChannel)
          }
          runtimes={personas.acpRuntimesQuery.data ?? []}
          runtimesLoading={personas.acpRuntimesQuery.isLoading}
        />
      ) : null}
      {personas.createdAgent ? (
        <SecretRevealDialog
          attachmentFailure={personas.attachmentFailure}
          created={personas.createdAgent}
          isRetryingAttachment={personas.isRetryingAttachment}
          onOpenChange={(open) => {
            if (!open) personas.dismissCreatedAgent();
          }}
          onRetryAttachment={() => {
            void personas.retryAttachment();
          }}
        />
      ) : null}
    </>
  );
}
