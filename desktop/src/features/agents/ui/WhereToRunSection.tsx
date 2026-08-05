import { AlertTriangle } from "lucide-react";
import * as React from "react";

import { useBackendProvidersQuery } from "@/features/agents/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import { probeBackendProvider } from "@/shared/api/tauri";

import { ProviderConfigFields } from "./ProviderConfigFields";
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
  const backendProviders = useBackendProvidersQuery().data ?? [];
  const [probeError, setProbeError] = React.useState<string | null>(null);
  const isProviderMode = draft.runOn !== "local";
  const channelsQuery = useChannelsQuery({ enabled: isProviderMode });
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
  const providerSchema = draft.probedProvider?.config_schema as
    | Record<string, unknown>
    | undefined;
  const roomProperty = (providerSchema?.properties as
    | Record<string, Record<string, unknown>>
    | undefined)?.rooms;
  const hasRoomPicker = roomProperty?.format === "buzz-room-picker";
  const discoverableRooms = (channelsQuery.data ?? []).filter(
    (channel) => channel.channelType !== "dm" && !channel.archivedAt,
  );
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
              {provider.id === "openclaw" ? "OpenClaw" : provider.id}
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
          {draft.probedProvider?.enrollment && !hasRoomPicker ? (
            <p className="rounded-2xl border border-primary/30 bg-primary/5 px-4 py-3 text-sm">
              This provider uses a one-time enrollment import. Buzz Desktop
              hands the agent identity to the trusted provider and keeps no
              runtime connection to the remote host.
            </p>
          ) : null}
          {hasRoomPicker ? (
            <div className="space-y-2">
              <label className="text-sm font-medium">
                Buzz rooms <span className="text-destructive">*</span>
              </label>
              <p className="text-xs text-muted-foreground">
                Select the rooms this provider should receive.
              </p>
              <div className="max-h-48 space-y-1 overflow-y-auto rounded-md border border-input p-2">
                {discoverableRooms.length === 0 ? (
                  <p className="p-2 text-sm text-muted-foreground">
                    No rooms available.
                  </p>
                ) : (
                  discoverableRooms.map((channel) => {
                    const selected = (draft.providerConfig.rooms ?? "")
                      .split(",")
                      .filter(Boolean)
                      .includes(channel.id);
                    return (
                      <label
                        className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-muted"
                        key={channel.id}
                      >
                        <input
                          checked={selected}
                          disabled={isPending}
                          onChange={() => {
                            const ids = new Set(
                              (draft.providerConfig.rooms ?? "")
                                .split(",")
                                .filter(Boolean),
                            );
                            if (selected) ids.delete(channel.id);
                            else ids.add(channel.id);
                            onDraftChange({
                              ...draft,
                              providerConfig: {
                                ...draft.providerConfig,
                                rooms: [...ids].join(","),
                              },
                            });
                          }}
                          type="checkbox"
                        />
                        <span>{channel.name}</span>
                      </label>
                    );
                  })
                )}
              </div>
            </div>
          ) : null}
          {draft.probedProvider?.config_schema ? (
            <ProviderConfigFields
              config={draft.providerConfig}
              onChange={(providerConfig) =>
                onDraftChange({ ...draft, providerConfig })
              }
              schema={draft.probedProvider.config_schema}
              excludeKeys={hasRoomPicker ? ["rooms"] : []}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
