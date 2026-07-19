import type * as React from "react";
import type { InheritedDefault } from "./bakedEnvHelpers";
import { getPersonaProviderOptions } from "./agentConfigOptions";
import { Button } from "@/shared/ui/button";

function providerLabel(providerId: string) {
  const option = getPersonaProviderOptions("", "buzz-agent").find(
    (candidate) => candidate.id === providerId,
  );
  return option?.label ?? providerId;
}

export function formatAiDefaultsSummary({
  provider,
  model,
}: {
  provider: InheritedDefault;
  model: InheritedDefault;
}) {
  const parts = [
    provider.value ? providerLabel(provider.value) : null,
    model.value || null,
  ].filter((value): value is string => Boolean(value));

  return parts.length > 0 ? parts.join(" · ") : "Not configured";
}

export function AgentAiDefaultsNotice({
  onEditDefaults,
  triggerRef,
  explicitModel,
  explicitProvider,
  inheritedModel,
  inheritedProvider,
}: {
  onEditDefaults: () => void;
  triggerRef?: React.Ref<HTMLButtonElement>;
  explicitModel: string;
  explicitProvider: string;
  inheritedModel: InheritedDefault;
  inheritedProvider: InheritedDefault;
}) {
  const provider = explicitProvider.trim() || inheritedProvider.value;
  const model = explicitModel.trim() || inheritedModel.value;

  return (
    <div className="space-y-1" data-testid="agent-ai-defaults-notice">
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 text-sm">
        <dt className="text-muted-foreground">Provider</dt>
        <dd className="truncate text-foreground">
          {provider ? providerLabel(provider) : "Not configured"}
        </dd>
        <dt className="text-muted-foreground">Model</dt>
        <dd className="truncate text-foreground">
          {model || "Not configured"}
        </dd>
      </dl>
      <Button
        className="h-auto px-0 py-1"
        data-testid="edit-ai-defaults"
        onClick={onEditDefaults}
        ref={triggerRef}
        size="xs"
        type="button"
        variant="link"
      >
        Edit global defaults
      </Button>
    </div>
  );
}
