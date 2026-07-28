import { AlertTriangle, ExternalLink, Server, Sparkles } from "lucide-react";
import * as React from "react";

import { useBackendProvidersQuery } from "@/features/agents/hooks";
import { probeBackendProvider } from "@/shared/api/tauri";
import { openUrl } from "@tauri-apps/plugin-opener";

import { ProviderConfigFields } from "./ProviderConfigFields";
import { emptyWhereToRunDraft, type WhereToRunDraft } from "./whereToRunIntent";

const CRABBOX_DOCS_URL = "https://crabbox.sh/";
const CRABBOX_INSTALL_HINT = "just install-backend-crabbox";

/** Run destination for a managed agent: this computer or a discovered remote backend. */
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
  const hasRemoteBackends = backendProviders.length > 0;
  const selectedBinaryPath = selectedBackendProvider?.binaryPath ?? null;
  const selectedProviderId = selectedBackendProvider?.id ?? null;

  // Keep a ref so the probe completion callback always sees the latest draft
  // without re-running when the user edits config fields.
  const draftRef = React.useRef(draft);
  draftRef.current = draft;

  React.useEffect(() => {
    if (!isProviderMode || !selectedBinaryPath || !selectedProviderId) {
      setProbeError(null);
      return;
    }
    let cancelled = false;
    setProbeError(null);
    void probeBackendProvider(selectedBinaryPath)
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
        // Only apply probe results if the user is still on this provider.
        if (draftRef.current.runOn !== selectedProviderId) return;
        onDraftChange({
          ...draftRef.current,
          runOn: selectedProviderId,
          probedProvider: result,
          providerConfig: defaults,
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
  }, [isProviderMode, onDraftChange, selectedBinaryPath, selectedProviderId]);

  // If the selected remote backend disappeared from PATH mid-dialog, snap back.
  React.useEffect(() => {
    if (isProviderMode && hasRemoteBackends && !selectedBackendProvider) {
      onDraftChange(emptyWhereToRunDraft);
    }
  }, [
    hasRemoteBackends,
    isProviderMode,
    onDraftChange,
    selectedBackendProvider,
  ]);

  const displayName =
    draft.probedProvider?.name?.trim() ||
    selectedBackendProvider?.name?.trim() ||
    selectedBackendProvider?.id ||
    "provider";
  const description =
    draft.probedProvider?.description?.trim() ||
    selectedBackendProvider?.description?.trim() ||
    null;

  return (
    <div className="space-y-4" data-testid="agent-where-to-run">
      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="agent-run-on">
          Run on
        </label>
        <p className="text-xs text-muted-foreground">
          Local runs on this computer. Remote backends spin the agent up
          elsewhere and keep it connected to your Buzz relay — same identity,
          different machine.
        </p>
        <select
          className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
          data-testid="agent-run-on"
          disabled={isPending}
          id="agent-run-on"
          onChange={(event) =>
            onDraftChange({
              ...emptyWhereToRunDraft,
              runOn: event.target.value,
            })
          }
          value={hasRemoteBackends ? draft.runOn : "local"}
        >
          <option value="local">This computer</option>
          {backendProviders.map((provider) => (
            <option
              key={provider.id}
              value={provider.id}
              data-testid={`agent-run-on-option-${provider.id}`}
            >
              {provider.name?.trim() || provider.id}
            </option>
          ))}
        </select>
      </div>

      {!hasRemoteBackends ? (
        <div
          className="flex gap-3 rounded-2xl border border-border/60 bg-muted/20 px-4 py-3"
          data-testid="agent-where-to-run-empty"
        >
          <Sparkles className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 space-y-2 text-sm">
            <p className="font-medium">Want agents on a remote box?</p>
            <p className="text-muted-foreground">
              Install the Crabbox backend once, restart Desktop, and{" "}
              <span className="font-medium text-foreground">Crabbox</span>{" "}
              appears here. Buzz still owns the agent identity and relay —
              Crabbox only hosts the harness.
            </p>
            <pre className="overflow-x-auto rounded-lg border border-border/50 bg-background px-3 py-2 font-mono text-xs">
              {CRABBOX_INSTALL_HINT}
            </pre>
            <button
              className="inline-flex items-center gap-1.5 text-xs font-medium text-primary hover:underline"
              onClick={() => {
                void openUrl(CRABBOX_DOCS_URL).catch(() => {
                  window.open(
                    CRABBOX_DOCS_URL,
                    "_blank",
                    "noopener,noreferrer",
                  );
                });
              }}
              type="button"
            >
              Crabbox docs
              <ExternalLink className="h-3 w-3" />
            </button>
          </div>
        </div>
      ) : null}

      {isProviderMode && selectedBackendProvider ? (
        <div className="space-y-4">
          {description ? (
            <div className="flex gap-3 rounded-2xl border border-border/60 bg-muted/30 px-4 py-3">
              <Server className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
              <div className="space-y-1 text-sm">
                <p className="font-medium">{displayName}</p>
                <p className="text-muted-foreground">{description}</p>
              </div>
            </div>
          ) : null}
          <div className="flex gap-3 rounded-2xl border border-warning/30 bg-warning-bg px-4 py-3">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
            <p className="text-sm text-warning">
              <span className="font-medium">{displayName}</span> (
              <span className="font-mono font-medium">
                {selectedBackendProvider.binaryPath}
              </span>
              ) will receive this agent&apos;s private key so it can sign as
              the agent on your relay. Only use backends you trust. Deleting
              the agent asks the backend to release remote capacity.
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
