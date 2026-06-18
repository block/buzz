import * as React from "react";
import { ChevronDown, Cpu } from "lucide-react";

import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import { cn } from "@/shared/lib/cn";

import {
  meshStartNode,
  meshStopNode,
  meshInstalledModels,
} from "@/shared/api/tauriMesh";
import type { MeshModelOption, MeshNodeStatus } from "@/shared/api/tauriMesh";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { classifyModelRef, modelRefHintLabel } from "../classifyModelRef";
import { useMeshNodeStatus } from "../hooks/useMeshNodeStatus";

const MODEL_DRAFT_STORAGE_KEY = "buzz.mesh-compute.share.model.v1";
const MAX_VRAM_DRAFT_STORAGE_KEY = "buzz.mesh-compute.share.max-vram-gb.v1";

function readDraft(key: string): string {
  try {
    return window.localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

function writeDraft(key: string, value: string): void {
  try {
    if (value === "") {
      window.localStorage.removeItem(key);
    } else {
      window.localStorage.setItem(key, value);
    }
  } catch {
    // Ignore unavailable/full storage; the input still works for this session.
  }
}

/**
 * Settings → Compute → Share compute.
 *
 * One toggle, one model field, an "Already installed" picklist, an Advanced
 * group. Honest copy throughout — no kind:30621, no "endpoint id", no raw
 * mesh knobs.
 *
 * The architectural-trust footer is load-bearing: it tells a privacy-aware
 * user that *not publishing* is enforced by the build, not by a default they
 * have to verify. (Source of the invariants this copy claims: Max's no-leak
 * builder defaults — publish=false, no Nostr relays, no auto-discovery.)
 */
export function MeshComputeSettingsCard() {
  const { status, error, refresh } = useMeshNodeStatus();
  const [installedModels, setInstalledModels] = React.useState<
    MeshModelOption[]
  >([]);
  const [modelInput, setModelInput] = React.useState(() =>
    readDraft(MODEL_DRAFT_STORAGE_KEY),
  );
  const [maxVramGb, setMaxVramGb] = React.useState<string>(() =>
    readDraft(MAX_VRAM_DRAFT_STORAGE_KEY),
  );
  const [advancedOpen, setAdvancedOpen] = React.useState(false);
  const [actionInFlight, setActionInFlight] = React.useState(false);
  const [actionError, setActionError] = React.useState<string | null>(null);

  // Fetch installed models. Called on mount and whenever the running state
  // changes (a fresh start may have downloaded a new model). Stale-tolerant —
  // the picklist is a convenience, not load-bearing.
  const refreshInstalled = React.useCallback(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await meshInstalledModels();
        if (!cancelled) setInstalledModels(list);
      } catch {
        // Non-fatal — picklist just stays empty; user can still type a ref.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: status?.state is the intentional trigger — re-fetch installed models when the node transitions (a fresh start may have downloaded a new model)
  React.useEffect(() => refreshInstalled(), [refreshInstalled, status?.state]);

  // Mirror the running node's modelId back into the field so the card shows
  // what's actually being served, even after a fresh app load.
  React.useEffect(() => {
    if (status?.state === "running" && status.modelId && modelInput === "") {
      setModelInput(status.modelId);
      writeDraft(MODEL_DRAFT_STORAGE_KEY, status.modelId);
    }
  }, [status?.state, status?.modelId, modelInput]);

  const isOn = status?.state === "running" || status?.state === "starting";
  const controlsDisabled = isOn || actionInFlight;
  const refClass = classifyModelRef(modelInput);
  const refHint = modelRefHintLabel(refClass);
  const canStart =
    refClass.kind !== "unknown" &&
    !actionInFlight &&
    status?.state !== "starting";

  async function handleToggle(next: boolean) {
    setActionError(null);
    setActionInFlight(true);
    try {
      if (next) {
        const maxVram =
          maxVramGb.trim() === "" ? undefined : Number.parseFloat(maxVramGb);
        await meshStartNode({
          mode: "serve",
          modelId: modelInput.trim() || undefined,
          maxVramGb:
            typeof maxVram === "number" && !Number.isNaN(maxVram)
              ? maxVram
              : undefined,
        });
      } else {
        await meshStopNode();
      }
      refresh();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setActionInFlight(false);
    }
  }

  return (
    <section className="min-w-0" data-testid="settings-mesh-share-compute">
      <div className="mb-12 min-w-0">
        <h2 className="text-2xl font-semibold tracking-tight">Share compute</h2>
        <p className="text-base font-normal text-muted-foreground">
          Share this machine with your relay. When on, other members can run
          their agents here.
        </p>
      </div>

      {error ? (
        <p className="mb-3 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive">
          Couldn't load mesh status: {error}
        </p>
      ) : null}
      {actionError ? (
        <p className="mb-3 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {actionError}
        </p>
      ) : null}

      <SettingsOptionGroup>
        <SettingsOptionRow>
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="mesh-share-compute-toggle"
            >
              Share this machine
            </label>
            <StatusLine status={status} />
          </div>
          <Switch
            checked={isOn}
            data-testid="mesh-share-compute-toggle"
            disabled={actionInFlight || (!isOn && !canStart)}
            id="mesh-share-compute-toggle"
            onCheckedChange={handleToggle}
          />
        </SettingsOptionRow>

        <div className="px-4 pb-4 pt-5">
          <label
            className="mb-3 flex items-center gap-2 text-sm font-medium"
            htmlFor="mesh-share-compute-model"
          >
            <Cpu className="h-4 w-4 text-muted-foreground" />
            Model
          </label>
          <div className="flex flex-col gap-2">
            <Input
              data-testid="mesh-share-compute-model"
              disabled={controlsDisabled}
              id="mesh-share-compute-model"
              onChange={(e) => {
                const next = e.target.value;
                setModelInput(next);
                writeDraft(MODEL_DRAFT_STORAGE_KEY, next);
              }}
              placeholder="Qwen3-8B-Q4_K_M or hf://meshllm/qwen3-8b@main"
              value={modelInput}
            />
            {refHint ? (
              <p className="text-sm font-normal text-muted-foreground">
                {refHint}
              </p>
            ) : (
              <p className="text-sm font-normal text-muted-foreground">
                Catalog name, HuggingFace ref, or a local file path.
              </p>
            )}
            {installedModels.length > 0 ? (
              <div className="mt-1">
                <p className="text-sm font-normal text-muted-foreground">
                  Already installed on this machine:
                </p>
                <ul
                  className="mt-1 flex flex-wrap gap-1.5"
                  data-testid="mesh-share-compute-installed-list"
                >
                  {installedModels.map((m) => (
                    <li key={m.id}>
                      <button
                        className="rounded border border-border/60 bg-muted/20 px-2 py-0.5 text-sm hover:bg-muted/40 disabled:cursor-not-allowed disabled:opacity-50"
                        disabled={controlsDisabled}
                        onClick={() => {
                          setModelInput(m.id);
                          writeDraft(MODEL_DRAFT_STORAGE_KEY, m.id);
                        }}
                        type="button"
                      >
                        {m.name ?? m.id}
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
          </div>
        </div>

        <details
          className="px-4 py-3"
          onToggle={(e) =>
            setAdvancedOpen((e.target as HTMLDetailsElement).open)
          }
          open={advancedOpen}
        >
          <summary className="flex cursor-pointer items-center gap-1.5 text-sm font-medium text-foreground">
            <ChevronDown
              className={cn(
                "h-4 w-4 text-muted-foreground transition-transform",
                advancedOpen ? "rotate-0" : "-rotate-90",
              )}
            />
            Advanced
          </summary>
          <div className="mt-3 flex flex-col gap-2">
            <label className="text-sm font-medium" htmlFor="mesh-vram">
              Max VRAM (GB)
            </label>
            <Input
              data-testid="mesh-share-compute-vram"
              id="mesh-vram"
              inputMode="decimal"
              onChange={(e) => {
                const next = e.target.value;
                setMaxVramGb(next);
                writeDraft(MAX_VRAM_DRAFT_STORAGE_KEY, next);
              }}
              placeholder="No limit"
              value={maxVramGb}
            />
            {status?.consoleUrl ? (
              <p className="text-sm font-normal text-muted-foreground">
                Debug console:{" "}
                <a
                  className="underline"
                  href={status.consoleUrl}
                  rel="noreferrer"
                  target="_blank"
                >
                  {status.consoleUrl}
                </a>
              </p>
            ) : null}
          </div>
        </details>
      </SettingsOptionGroup>

      <p className="mt-3 rounded-lg bg-muted/30 px-3 py-2 text-sm font-normal text-muted-foreground">
        Buzz will not publish your machine to public Nostr relays, auto-discover
        other networks, or share your endpoint outside this relay's members.
        Only members of this relay can dial in.
      </p>
    </section>
  );
}

/**
 * Renders the lifecycle/health text under the toggle. Maps Max's `state` ×
 * `health` matrix to honest copy — no "starting…" stuck forever when mesh
 * is actually downloading weights or has failed.
 */
function StatusLine({ status }: { status: MeshNodeStatus | null }) {
  if (!status) {
    return <p className="text-sm text-muted-foreground">Loading…</p>;
  }
  const { state, health, modelId, modelName } = status;
  const modelLabel = modelName ?? modelId ?? "";

  if (state === "off") {
    return (
      <p className="text-sm text-muted-foreground">Not sharing right now.</p>
    );
  }
  if (state === "starting") {
    const reason =
      health.status === "degraded" || health.status === "failed"
        ? health.reason
        : "Starting…";
    return <p className="text-sm text-muted-foreground">{reason}</p>;
  }
  if (state === "running") {
    if (health.status === "failed") {
      return (
        <p className="text-sm text-destructive">
          Couldn't load: {health.reason}
        </p>
      );
    }
    if (health.status === "degraded") {
      return (
        <p className="text-sm text-amber-600 dark:text-amber-400">
          Active{modelLabel ? ` — ${modelLabel}` : ""}. {health.reason}
        </p>
      );
    }
    return (
      <p className="text-sm text-muted-foreground">
        Active{modelLabel ? ` — serving ${modelLabel}` : ""}.
      </p>
    );
  }
  if (state === "stopping") {
    return <p className="text-sm text-muted-foreground">Stopping…</p>;
  }
  if (state === "failed") {
    const reason =
      health.status === "failed" || health.status === "degraded"
        ? health.reason
        : "Couldn't start.";
    return <p className="text-sm text-destructive">{reason}</p>;
  }
  return null;
}
