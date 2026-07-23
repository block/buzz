import * as React from "react";

import {
  consumePendingOpenCreateAgent,
  subscribeOpenCreateAgent,
  type OpenCreateAgentOptions,
} from "@/features/agents/openCreateAgentEvent";
import { AgentDialog } from "./AgentDialog";
import { SecretRevealDialog } from "./SecretRevealDialog";
import { usePersonaActions } from "./usePersonaActions";

/** App-level create flow so contextual entry points do not navigate away. */
export function RequestedAgentCreateDialogs() {
  const personas = usePersonaActions();
  const [targetChannel, setTargetChannel] = React.useState<{
    id: string;
    name: string;
  } | null>(null);
  const [isOpen, setIsOpen] = React.useState(false);

  const openCreate = React.useEffectEvent((options: OpenCreateAgentOptions) => {
    personas.prepareCreate();
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

  return (
    <>
      {isOpen ? (
        <AgentDialog
          definitionError={
            personas.createPersonaMutation.error instanceof Error
              ? personas.createPersonaMutation.error
              : null
          }
          isDefinitionPending={personas.isPending}
          mode="definition"
          onOpenChange={(open) => {
            if (!open) {
              setIsOpen(false);
              setTargetChannel(null);
            }
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
