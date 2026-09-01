import * as React from "react";

import {
  consumePendingOpenCreateAgent,
  subscribeOpenCreateAgent,
  type OpenCreateAgentOptions,
} from "@/features/agents/openCreateAgentEvent";
import type { CreatePersonaInput } from "@/shared/api/types";
import { AgentDialog } from "./AgentDialog";
import { usePersonaActions } from "./usePersonaActions";

/** App-level create flow so contextual entry points do not navigate away. */
export function RequestedAgentCreateDialogs() {
  const personas = usePersonaActions();
  const [targetChannel, setTargetChannel] = React.useState<{
    id: string;
    name: string;
  } | null>(null);
  const [isOpen, setIsOpen] = React.useState(false);
  const [initialValues, setInitialValues] =
    React.useState<CreatePersonaInput | null>(null);

  const openCreate = React.useEffectEvent((options: OpenCreateAgentOptions) => {
    personas.prepareCreate();
    setInitialValues(
      options.preset === "community-mesh"
        ? {
            displayName: "Community agent",
            avatarUrl: "",
            systemPrompt: "",
            runtime: "buzz-agent",
            provider: "relay-mesh",
            model: "auto",
          }
        : null,
    );
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
          initialValues={initialValues}
          isDefinitionPending={personas.isPending}
          mode="definition"
          onOpenChange={(open) => {
            if (!open) {
              setIsOpen(false);
              setTargetChannel(null);
              setInitialValues(null);
            }
          }}
          onSubmitDefinition={(input, intent, backendIntent) =>
            personas.handleSubmit(input, intent, backendIntent, targetChannel)
          }
          runtimes={personas.acpRuntimesQuery.data ?? []}
          runtimeCatalogStatus={
            personas.acpRuntimesQuery.isLoading
              ? "loading"
              : personas.acpRuntimesQuery.isError
                ? "error"
                : "ready"
          }
        />
      ) : null}
    </>
  );
}
