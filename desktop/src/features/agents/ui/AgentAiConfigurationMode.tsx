import type * as React from "react";
import { useTranslation } from "@/i18n";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import type { AgentAiConfigurationMode } from "./agentAiConfigurationPolicy";
import { AgentAiDefaultsNotice } from "./AgentAiDefaults";
import type { InheritedDefault } from "./bakedEnvHelpers";

export type { AgentAiConfigurationMode } from "./agentAiConfigurationPolicy";

export function HarnessModelDefaultNotice({
  harness,
  model,
}: {
  harness: string;
  model?: string | null;
}) {
  const { t } = useTranslation();
  return (
    <dl
      className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 text-sm"
      data-testid="agent-harness-defaults-notice"
    >
      <dt className="text-muted-foreground">{t("agents.ai-config.harness")}</dt>
      <dd className="truncate text-foreground">
        {harness || t("agents.ai-config.not-configured")}
      </dd>
      <dt className="text-muted-foreground">{t("mesh-compute.card.model")}</dt>
      <dd className="truncate text-foreground">
        {model?.trim() || t("agents.ai-config.harness-default")}
      </dd>
    </dl>
  );
}

export function AgentCreateAiDefaultsSummary({
  canChooseProvider,
  harness,
  inheritedModel,
  inheritedProvider,
  isConfigured,
  model,
  onEditDefaults,
  triggerRef,
}: {
  canChooseProvider: boolean;
  harness: string;
  inheritedModel: InheritedDefault;
  inheritedProvider: InheritedDefault;
  isConfigured: boolean;
  model?: string | null;
  onEditDefaults: () => void;
  triggerRef?: React.Ref<HTMLButtonElement>;
}) {
  return canChooseProvider ? (
    <AgentAiDefaultsNotice
      isConfigured={isConfigured}
      onEditDefaults={onEditDefaults}
      triggerRef={triggerRef}
      explicitModel=""
      explicitProvider=""
      harness={harness}
      inheritedModel={inheritedModel}
      inheritedProvider={inheritedProvider}
    />
  ) : (
    <HarnessModelDefaultNotice harness={harness} model={model} />
  );
}

export function AgentAiConfigurationModeField({
  mode,
  needsProviderSelection = true,
  onModeChange,
}: {
  mode: AgentAiConfigurationMode;
  needsProviderSelection?: boolean;
  onModeChange: (mode: AgentAiConfigurationMode) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-1.5">
      <p className="text-sm font-medium text-foreground">
        {t("agents.ai-config.section")}
      </p>
      <Tabs
        onValueChange={(value) =>
          onModeChange(value as AgentAiConfigurationMode)
        }
        value={mode}
      >
        <TabsList className="relative isolate grid h-9 w-full grid-cols-2 overflow-hidden rounded-lg bg-muted p-0.5">
          <div
            aria-hidden="true"
            className="absolute bottom-0.5 left-0.5 top-0.5 z-0 rounded-md bg-background shadow-sm transition-transform duration-[250ms] ease-out"
            style={{
              transform: `translateX(${mode === "custom" ? 100 : 0}%)`,
              width: "calc((100% - 4px) / 2)",
            }}
          />
          <TabsTrigger
            className="relative z-10 h-full rounded-md bg-transparent text-xs font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:shadow-none"
            value="defaults"
          >
            {needsProviderSelection
              ? t("agents.ai-config.use-agent-defaults")
              : t("agents.ai-config.use-harness-defaults")}
          </TabsTrigger>
          <TabsTrigger
            className="relative z-10 h-full rounded-md bg-transparent text-xs font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:shadow-none"
            value="custom"
          >
            {t("agents.ai-config.customize")}
          </TabsTrigger>
        </TabsList>
      </Tabs>
    </div>
  );
}
