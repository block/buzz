import { AlertTriangle } from "lucide-react";
import * as React from "react";

import { useTranslation } from "@/i18n";
import { useBackendProvidersQuery } from "@/features/agents/hooks";
import { probeBackendProvider } from "@/shared/api/tauri";

import { ProviderConfigFields } from "./ProviderConfigFields";
import { PersonaDropdownField } from "./PersonaDropdownField";
import {
  applyProbeResult,
  emptyWhereToRunDraft,
  type WhereToRunDraft,
} from "./whereToRunIntent";

/** Optional remote-backend selector. Buzz shared compute is an LLM provider, not a run destination. */
export function WhereToRunSection({
  draft,
  isPending,
  onDraftChange,
}: {
  draft: WhereToRunDraft;
  isPending: boolean;
  onDraftChange: (next: WhereToRunDraft) => void;
}) {
  const { t } = useTranslation();
  const backendProviders = useBackendProvidersQuery().data ?? [];
  const [probeError, setProbeError] = React.useState<string | null>(null);
  const runOnOptions = React.useMemo(
    () => [
      { label: t("agents.run-on.this-computer"), value: "local" },
      ...backendProviders.map((provider) => ({
        label: provider.id,
        value: provider.id,
      })),
    ],
    [backendProviders, t],
  );
  const isProviderMode = draft.runOn !== "local";
  const selectedBackendProvider = React.useMemo(
    () =>
      backendProviders.find((provider) => provider.id === draft.runOn) ?? null,
    [backendProviders, draft.runOn],
  );

  // Latest-state seam for probe resolution: an Effect Event always sees the
  // draft as it is *now*. Without this, the probe promise closes over the
  // draft from probe start, and anything typed while the probe was in flight
  // gets thrown away when it resolves (a second, subtler Typewriter Eraser).
  const applyProbe = React.useEffectEvent(
    (result: Awaited<ReturnType<typeof probeBackendProvider>>) => {
      onDraftChange(applyProbeResult(draft, result));
    },
  );

  // Probe once per provider *selection*, keyed on the provider's stable
  // path — never on the draft. Depending on the draft made every keystroke
  // refire the probe, and each resolution reset providerConfig to schema
  // defaults, which erased what the user was typing (the Typewriter Eraser)
  // and spawned the provider binary in a loop for as long as the dialog was
  // open. Keying on the path (not the provider object) also keeps a
  // providers-query refresh from reprobing an unchanged selection.
  const selectedBinaryPath = isProviderMode
    ? (selectedBackendProvider?.binaryPath ?? null)
    : null;
  React.useEffect(() => {
    if (!selectedBinaryPath || draft.probedProvider) {
      setProbeError(null);
      return;
    }
    let cancelled = false;
    setProbeError(null);
    void probeBackendProvider(selectedBinaryPath)
      .then((result) => {
        if (cancelled) return;
        applyProbe(result);
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setProbeError(error instanceof Error ? error.message : String(error));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [selectedBinaryPath, draft.probedProvider]);

  if (backendProviders.length === 0) return null;

  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="agent-run-on">
          {t("agents.run-on.label")}
        </label>
        <PersonaDropdownField
          disabled={isPending}
          id="agent-run-on"
          onValueChange={(runOn) =>
            onDraftChange({
              ...emptyWhereToRunDraft,
              runOn,
            })
          }
          options={runOnOptions}
          placeholder={t("agents.where-to-run.placeholder")}
          value={draft.runOn}
        />
      </div>

      {isProviderMode && selectedBackendProvider ? (
        <div className="space-y-4">
          <div className="flex gap-3 rounded-2xl border border-warning/30 bg-warning-bg px-4 py-3">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
            <p className="text-sm text-warning">
              {t("agents.where-to-run.key-warning", {
                path: selectedBackendProvider.binaryPath,
              })}
            </p>
          </div>
          {probeError ? (
            <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
              {t("agents.where-to-run.probe-failed", { error: probeError })}
            </p>
          ) : null}
          {draft.probedProvider?.config_schema ? (
            <ProviderConfigFields
              config={draft.providerConfig}
              onChange={(providerConfig) =>
                onDraftChange({ ...draft, providerConfig })
              }
              schema={draft.probedProvider.config_schema}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
