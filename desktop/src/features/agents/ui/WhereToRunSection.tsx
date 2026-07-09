import { AlertTriangle } from "lucide-react";
import * as React from "react";

import { useBackendProvidersQuery } from "@/features/agents/hooks";
import { RelayMeshAgentSection } from "@/features/mesh-compute/ui/RelayMeshAgentSection";
import { probeBackendProvider } from "@/shared/api/tauri";

import { ProviderConfigFields } from "./ProviderConfigFields";
import { emptyWhereToRunDraft, type WhereToRunDraft } from "./whereToRunIntent";

/**
 * "Where to run" selector for the definition-create start flow (B5): local
 * (default), a discovered backend provider, or the relay mesh. Owns the
 * probe/config/mesh-target draft internally and reports it upward via
 * `onDraftChange`; the parent resolves it into a BackendIntent at submit
 * (`resolveBackendIntent`) and gates the submit button
 * (`canSubmitWhereToRun`).
 *
 * Only rendered while the start-after-create toggle is ON — "where to run"
 * is instance state, and with the toggle off no instance exists. The parent
 * discards the draft at submit when the toggle is off (the stale-intent
 * guard), so a selection made before toggling off can never silently ride
 * a definition-only create.
 *
 * Honest-copy note: unlike the legacy create dialog, the mesh preset never
 * overwrites the definition's fields — only the minted instance carries the
 * mesh commands/env — so RelayMeshAgentSection's override warning gets an
 * empty `current` and stays silent by construction.
 */
export function WhereToRunSection({
  draft,
  isPending,
  onDraftChange,
}: {
  draft: WhereToRunDraft;
  isPending: boolean;
  onDraftChange: (next: WhereToRunDraft) => void;
}) {
  const backendProvidersQuery = useBackendProvidersQuery();
  const backendProviders = backendProvidersQuery.data ?? [];
  const [probeError, setProbeError] = React.useState<string | null>(null);

  const isProviderMode = draft.runOn !== "local" && draft.runOn !== "mesh";
  const selectedBackendProvider = React.useMemo(
    () => backendProviders.find((p) => p.id === draft.runOn) ?? null,
    [backendProviders, draft.runOn],
  );

  // Latest draft, updated synchronously on every emit — RelayMeshAgentSection
  // fires onTargetChange and onModelIdChange back-to-back in one event
  // handler, before React re-renders, so reading the prop alone would let the
  // second callback stamp stale sibling fields over the first one's update.
  const draftRef = React.useRef(draft);
  draftRef.current = draft;
  const emit = React.useCallback(
    (next: WhereToRunDraft) => {
      draftRef.current = next;
      onDraftChange(next);
    },
    [onDraftChange],
  );

  // Probe the provider when a non-local, non-mesh backend is selected.
  // Mirrors the legacy dialog: config defaults from the schema are seeded so
  // unchanged defaults are included in the submit payload.
  React.useEffect(() => {
    if (!isProviderMode || !selectedBackendProvider) {
      setProbeError(null);
      return;
    }

    let cancelled = false;
    setProbeError(null);

    probeBackendProvider(selectedBackendProvider.binaryPath)
      .then((result) => {
        if (cancelled) return;
        const defaults: Record<string, string> = {};
        if (result.config_schema) {
          const props =
            (result.config_schema as Record<string, unknown>)?.properties ?? {};
          for (const [key, prop] of Object.entries(props) as [
            string,
            Record<string, unknown>,
          ][]) {
            if (prop.default != null) {
              defaults[key] = String(prop.default);
            }
          }
        }
        emit({
          ...draftRef.current,
          probedProvider: result,
          providerConfig: defaults,
        });
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setProbeError(err instanceof Error ? err.message : String(err));
        }
      });

    return () => {
      cancelled = true;
    };
  }, [isProviderMode, selectedBackendProvider, emit]);

  function handleRunOnChange(nextValue: string) {
    setProbeError(null);
    // Switching destination resets the destination-specific draft state so a
    // previous selection can't leak into the new one.
    emit({ ...emptyWhereToRunDraft, runOn: nextValue });
  }

  const useMesh = draft.runOn === "mesh";
  const showRunOnPicker = backendProviders.length > 0 || useMesh;

  return (
    <div className="space-y-4">
      {showRunOnPicker ? (
        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="agent-run-on">
            Run on
          </label>
          <select
            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
            disabled={isPending}
            id="agent-run-on"
            onChange={(e) => handleRunOnChange(e.target.value)}
            value={draft.runOn}
          >
            <option value="local">This computer</option>
            {backendProviders.map((p) => (
              <option key={p.id} value={p.id}>
                {p.id}
              </option>
            ))}
          </select>
        </div>
      ) : null}

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
              onChange={(config) =>
                emit({ ...draftRef.current, providerConfig: config })
              }
              schema={draft.probedProvider.config_schema}
            />
          ) : null}
        </div>
      ) : null}

      {!isProviderMode ? (
        <>
          <RelayMeshAgentSection
            current={{
              // The definition's own fields are never overwritten by the mesh
              // preset — only the minted instance carries it — so the override
              // warning has nothing to warn about.
              acpCommand: "",
              agentCommand: "",
              agentArgs: [],
              mcpCommand: "",
              model: null,
              envVars: {},
            }}
            modelId={draft.meshModelId}
            targetEndpointAddr={draft.meshTarget?.endpointAddr ?? ""}
            onModelIdChange={(nextId, patch) => {
              emit({
                ...draftRef.current,
                meshModelId: nextId,
                meshPatch: patch,
              });
            }}
            onTargetChange={(target) => {
              emit({ ...draftRef.current, meshTarget: target });
            }}
            onUseMeshChange={(next) => {
              emit(
                next
                  ? { ...emptyWhereToRunDraft, runOn: "mesh" }
                  : { ...emptyWhereToRunDraft },
              );
            }}
            useMesh={useMesh}
          />
          {useMesh ? (
            <p className="text-xs text-muted-foreground">
              The started instance runs on the mesh model; the agent profile
              keeps its own settings.
            </p>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
