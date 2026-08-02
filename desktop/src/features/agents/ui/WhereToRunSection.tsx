import { AlertTriangle, Boxes, Laptop, Server } from "lucide-react";
import * as React from "react";

import {
  useBackendProvidersQuery,
  useExecutionNodesQuery,
} from "@/features/agents/hooks";
import { probeBackendProvider } from "@/shared/api/tauri";
import { cn } from "@/shared/lib/cn";

import { ProviderConfigFields } from "./ProviderConfigFields";
import {
  applyProbeResult,
  emptyWhereToRunDraft,
  isExecutionNodeRunOn,
  type WhereToRunDraft,
} from "./whereToRunIntent";
import {
  deriveRunOnOptions,
  type RunOnAvailability,
  type RunOnOption,
} from "./whereToRunOptions";

const RUN_ON_ICONS: Record<RunOnOption["kind"], typeof Laptop> = {
  "execution-node": Server,
  local: Laptop,
  provider: Boxes,
};

// Node-liveness dots. Deliberately card-local rather than the app-wide
// presence palette: degraded reads as yellow here, and unavailable is a
// neutral gray — a dimmed green would still read as "alive".
const AVAILABILITY_DOT_CLASSES: Record<RunOnAvailability, string> = {
  connected: "bg-emerald-500",
  degraded: "bg-yellow-500",
  unavailable: "bg-muted-foreground",
};

function runOnCardAriaLabel(option: RunOnOption): string {
  const parts = [option.label];
  if (option.detail) parts.push(option.detail);
  if (option.availability && option.availability !== "connected") {
    parts.push(option.availability);
  }
  return parts.join(", ");
}

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
  // Provider binaries are deployment adapters (docker, k8s, …) discovered on
  // this machine; execution nodes are paired remote compute. Providers stay
  // behind a flag until the provider contract work lands.
  const providersEnabled =
    import.meta.env.DEV ||
    import.meta.env.VITE_ENABLE_BACKEND_PROVIDERS === "1";
  const backendProviders =
    useBackendProvidersQuery({ enabled: providersEnabled }).data ?? [];
  const executionNodes = useExecutionNodesQuery().data ?? [];
  const deployableExecutionNodes = executionNodes.filter((node) =>
    node.capabilities.includes("deploy"),
  );
  const [probeError, setProbeError] = React.useState<string | null>(null);
  // Scope the native radio group to this dialog instance; a bare string name
  // would couple create- and edit-dialog groups if both ever mount.
  const runOnGroupName = React.useId();
  const isExecutionNode = isExecutionNodeRunOn(draft.runOn);
  const isProviderMode =
    providersEnabled && draft.runOn !== "local" && !isExecutionNode;
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
    if (!selectedBinaryPath) {
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
  }, [selectedBinaryPath]);

  const options = deriveRunOnOptions({
    backendProviders,
    executionNodes: deployableExecutionNodes,
    providersEnabled,
    runOn: draft.runOn,
  });

  const selectRunOn = (value: string) => {
    // Re-selecting the current card must be a no-op: resetting the draft
    // would wipe an in-flight provider probe and its config.
    if (value === draft.runOn) return;
    onDraftChange({ ...emptyWhereToRunDraft, runOn: value });
  };

  // Hide only when there is nothing to choose AND nothing is chosen: an edit
  // dialog whose agent sits on a now-unannounced node or provider must still
  // render so the user can move it back to local. `options` always contains
  // "This computer" plus a fallback card for any non-local selection.
  if (options.length === 1 && draft.runOn === "local") return null;

  return (
    <div className="space-y-4">
      <fieldset className="space-y-1.5" id="agent-run-on">
        <legend className="text-sm font-medium">Run on</legend>
        <div className="grid grid-cols-[repeat(auto-fill,minmax(8.5rem,1fr))] gap-2">
          {options.map((option) => {
            const checked = option.value === draft.runOn;
            const Icon = RUN_ON_ICONS[option.kind];
            const interactive = option.selectable && !isPending;
            return (
              <label
                className={cn(
                  "relative flex min-h-20 flex-col items-center gap-1.5 rounded-lg border px-3 py-3 text-center transition-colors has-focus-visible:ring-2 has-focus-visible:ring-ring",
                  checked
                    ? "border-primary bg-primary/10 text-foreground"
                    : "border-border/70 text-muted-foreground",
                  interactive
                    ? cn(
                        "cursor-pointer",
                        checked
                          ? null
                          : "hover:border-border hover:text-foreground",
                      )
                    : "cursor-not-allowed",
                  option.selectable ? null : "opacity-50",
                  isPending ? "opacity-60" : null,
                )}
                key={option.value}
              >
                <input
                  aria-label={runOnCardAriaLabel(option)}
                  checked={checked}
                  className="sr-only"
                  disabled={isPending || !option.selectable}
                  name={runOnGroupName}
                  onChange={() => selectRunOn(option.value)}
                  type="radio"
                  value={option.value}
                />
                {option.availability ? (
                  <span
                    aria-hidden="true"
                    className={cn(
                      "absolute right-1.5 top-1.5 h-2 w-2 rounded-full",
                      AVAILABILITY_DOT_CLASSES[option.availability],
                      // The current-but-unavailable card stays selectable, so
                      // it skips the card-level 50% dim — fade its dot alone.
                      option.availability === "unavailable" && option.selectable
                        ? "opacity-50"
                        : null,
                    )}
                  />
                ) : null}
                <span className="flex flex-1 items-center justify-center">
                  <Icon aria-hidden="true" className="h-5 w-5 shrink-0" />
                </span>
                {/* Title is the bottom anchor on every card; the rare detail
                    line (edit-mode fallbacks) sits above it, never below. */}
                {option.detail ? (
                  <span className="block w-full min-w-0 truncate text-2xs text-muted-foreground">
                    {option.detail}
                  </span>
                ) : null}
                <span className="block w-full min-w-0 truncate text-xs font-medium">
                  {option.label}
                </span>
              </label>
            );
          })}
        </div>
      </fieldset>

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
