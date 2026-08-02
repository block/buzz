import { AlertTriangle } from "lucide-react";
import * as React from "react";

import { useBackendProvidersQuery } from "@/features/agents/hooks";
import { probeBackendProvider } from "@/shared/api/tauri";

import { ProviderConfigFields } from "./ProviderConfigFields";
import { emptyWhereToRunDraft, type WhereToRunDraft } from "./whereToRunIntent";

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
  const backendProviders = useBackendProvidersQuery().data ?? [];
  const [probeError, setProbeError] = React.useState<string | null>(null);
  const isProviderMode = draft.runOn !== "local";
  const selectedBackendProvider = React.useMemo(
    () =>
      backendProviders.find((provider) => provider.id === draft.runOn) ?? null,
    [backendProviders, draft.runOn],
  );

  React.useEffect(() => {
    if (!isProviderMode || !selectedBackendProvider) {
      setProbeError(null);
      return;
    }
    let cancelled = false;
    setProbeError(null);
    void probeBackendProvider(selectedBackendProvider.binaryPath)
      .then((result) => {
        if (cancelled) return;
        const defaults: Record<string, string> = {};
        const properties =
          (result.config_schema as Record<string, unknown> | undefined)
            ?.properties ?? {};
        for (const [key, property] of Object.entries(properties) as [
          string,
          Record<string, unknown>,
        ][]) {
          if (property.default != null)
            defaults[key] = String(property.default);
        }
        onDraftChange({
          ...draft,
          probedProvider: result,
          providerConfig: { ...defaults, ...draft.providerConfig },
          allowUnprobedProvider: false,
        });
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setProbeError(error instanceof Error ? error.message : String(error));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [draft, isProviderMode, onDraftChange, selectedBackendProvider]);

  if (backendProviders.length === 0) return null;

  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="agent-run-on">
          Run on
        </label>
        <select
          className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
          disabled={isPending}
          id="agent-run-on"
          onChange={(event) =>
            onDraftChange({
              ...emptyWhereToRunDraft,
              runOn: event.target.value,
            })
          }
          value={draft.runOn}
        >
          <option value="local">This computer</option>
          {backendProviders.map((provider) => (
            <option key={provider.id} value={provider.id}>
              {provider.id}
            </option>
          ))}
        </select>
      </div>

      {isProviderMode && selectedBackendProvider ? (
        <div className="space-y-4">
          <div className="flex gap-3 rounded-2xl border border-warning/30 bg-warning-bg px-4 py-3">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
            <p className="text-sm text-warning">
              This provider at{" "}
              <span className="font-mono font-medium">
                {selectedBackendProvider.binaryPath}
              </span>{" "}
              will receive your agent&apos;s private key. Only use providers
              from trusted sources.
            </p>
          </div>
          {probeError ? (
            <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
              Could not probe provider: {probeError}
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
