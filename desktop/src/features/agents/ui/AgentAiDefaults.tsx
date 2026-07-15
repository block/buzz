import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { InheritedDefault } from "./bakedEnvHelpers";
import { getPersonaProviderOptions } from "./personaDialogPickers";
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
  confirmNavigation = false,
  explicitModel,
  explicitProvider,
  inheritedModel,
  inheritedProvider,
}: {
  confirmNavigation?: boolean;
  explicitModel: string;
  explicitProvider: string;
  inheritedModel: InheritedDefault;
  inheritedProvider: InheritedDefault;
}) {
  const { goSettings } = useAppNavigation();
  const inheritsProvider = explicitProvider.trim().length === 0;
  const inheritsModel = explicitModel.trim().length === 0;

  const usesCustomConfig = !inheritsProvider && !inheritsModel;
  const requiredProviderMissing = inheritsProvider && !inheritedProvider.value;

  const inheritedParts = [
    inheritsProvider
      ? inheritedProvider.value
        ? `Provider ${providerLabel(inheritedProvider.value)}`
        : "Provider not configured"
      : null,
    inheritsModel
      ? inheritedModel.value
        ? `Model ${inheritedModel.value}`
        : "Model not configured"
      : null,
  ].filter((value): value is string => Boolean(value));

  return (
    <div
      className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-border/60 bg-muted/30 px-3 py-2"
      data-testid="agent-ai-defaults-notice"
    >
      <div className="min-w-0">
        <p className="text-xs font-medium text-foreground">
          {usesCustomConfig
            ? "Custom AI configuration"
            : requiredProviderMissing
              ? "AI defaults aren’t configured"
              : inheritsProvider && inheritsModel
                ? "Uses AI defaults"
                : "Partially uses AI defaults"}
        </p>
        <p className="text-xs text-muted-foreground">
          {usesCustomConfig
            ? "This agent won’t follow provider or model default changes."
            : requiredProviderMissing
              ? "Choose a provider in AI defaults to use this agent."
              : `${inheritedParts.join(" · ")}. Inherited fields follow future changes.`}
        </p>
      </div>
      <Button
        onClick={() => {
          if (
            confirmNavigation &&
            !window.confirm(
              "Leave this agent without saving? Your changes will be discarded.",
            )
          ) {
            return;
          }
          void goSettings("agents");
        }}
        size="xs"
        type="button"
        variant="link"
      >
        Edit AI defaults
      </Button>
    </div>
  );
}
