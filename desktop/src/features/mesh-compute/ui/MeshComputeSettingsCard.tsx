import * as React from "react";
import { ChevronDown, Cpu } from "lucide-react";

import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import { cn } from "@/shared/lib/cn";

import {
  meshStartNode,
  meshStopNode,
  meshInstalledModels,
  meshModelCatalog,
} from "@/shared/api/tauriMesh";
import type {
  MeshCatalogEntry,
  MeshModelCatalog,
  MeshModelOption,
  MeshNodeStatus,
} from "@/shared/api/tauriMesh";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import { classifyModelRef } from "../classifyModelRef";
import {
  downloadPercent,
  formatDownloadBytes,
  useMeshDownloadProgress,
} from "../hooks/useMeshDownloadProgress";
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
 * group. User-facing copy describes the shared-compute behavior without
 * exposing implementation protocols or raw mesh controls.
 */
export function MeshComputeSettingsCard() {
  const { status, error, refresh } = useMeshNodeStatus();
  const [installedModels, setInstalledModels] = React.useState<
    MeshModelOption[]
  >([]);
  const [catalog, setCatalog] = React.useState<MeshModelCatalog | null>(null);
  const [modelInput, setModelInput] = React.useState(() =>
    readDraft(MODEL_DRAFT_STORAGE_KEY),
  );
  const [maxVramGb, setMaxVramGb] = React.useState<string>(() =>
    readDraft(MAX_VRAM_DRAFT_STORAGE_KEY),
  );
  const [advancedOpen, setAdvancedOpen] = React.useState(false);
  const [actionInFlight, setActionInFlight] = React.useState(false);
  const [pendingAction, setPendingAction] = React.useState<
    "start" | "stop" | null
  >(null);
  const [actionError, setActionError] = React.useState<string | null>(null);
  const { progress: downloadProgress, reset: resetDownloadProgress } =
    useMeshDownloadProgress();

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

  // One-shot hardware-aware catalog fetch. Purely additive: when it fails
  // (stub build, survey error) the card falls back to the free-text field.
  // Keep an empty draft empty so the UI can explicitly ask the member to choose.
  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const value = await meshModelCatalog();
        if (cancelled) return;
        setCatalog(value);
      } catch {
        // Non-fatal — picker just doesn't render.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

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
  const canStart =
    refClass.kind !== "unknown" &&
    !actionInFlight &&
    status?.state !== "starting";

  async function handleToggle(next: boolean) {
    setActionError(null);
    setPendingAction(next ? "start" : "stop");
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
      setPendingAction(null);
      resetDownloadProgress();
    }
  }

  return (
    <section className="min-w-0" data-testid="settings-mesh-share-compute">
      <SettingsSectionHeader
        title="Share compute"
        description={
          <>
            Share this machine with your relay. When on, other members can run
            their agents here.
          </>
        }
      />

      {error ? (
        <p className="mb-3 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive">
          Couldn't check shared compute: {error}
        </p>
      ) : null}
      {actionError ? (
        <p className="mb-3 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {actionError}
        </p>
      ) : null}
      {downloadProgress ? (
        <DownloadProgressBar progress={downloadProgress} />
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
            <StatusLine pendingAction={pendingAction} status={status} />
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
            <p className="text-sm font-normal text-muted-foreground">
              Choose a suggested model below, or enter a model reference or
              local file. Buzz downloads remote models when sharing starts.
            </p>
            {catalog && catalog.entries.length > 0 ? (
              <CatalogPicker
                catalog={catalog}
                disabled={controlsDisabled}
                onPick={(name) => {
                  setModelInput(name);
                  writeDraft(MODEL_DRAFT_STORAGE_KEY, name);
                }}
                selected={modelInput.trim()}
              />
            ) : null}
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
        Only members of this relay can use this machine's shared compute.
      </p>
    </section>
  );
}

/**
 * Renders the lifecycle/health text under the toggle. Maps Max's `state` ×
 * `health` matrix to honest copy — no "starting…" stuck forever when mesh
 * is actually downloading weights or has failed.
 */
/**
 * Live model-download progress: name, bytes, percent bar. Rendered above the
 * option group while the backend streams mesh-download-progress events —
 * the answer to "it just greys out while downloading".
 */
function DownloadProgressBar({
  progress,
}: {
  progress: NonNullable<ReturnType<typeof useMeshDownloadProgress>["progress"]>;
}) {
  const percent = downloadPercent(progress);
  const bytes = formatDownloadBytes(progress);
  return (
    <div
      className="mb-3 rounded-lg bg-muted/30 px-3 py-2"
      data-testid="mesh-download-progress"
    >
      <div className="flex items-baseline justify-between gap-2 text-sm">
        <span className="min-w-0 truncate font-medium">
          {progress.status === "preparing" ? "Preparing" : "Downloading"}{" "}
          {progress.label}
        </span>
        <span className="shrink-0 text-muted-foreground">
          {percent != null ? `${percent}%` : bytes || "…"}
        </span>
      </div>
      {bytes && percent != null ? (
        <p className="mt-0.5 text-sm font-normal text-muted-foreground">
          {bytes}
        </p>
      ) : null}
      <div className="mt-1.5 h-1.5 w-full overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            "h-full rounded-full bg-primary transition-[width] duration-300",
            percent == null && "w-1/4 animate-pulse",
          )}
          style={percent != null ? { width: `${percent}%` } : undefined}
        />
      </div>
    </div>
  );
}

const FIT_LABEL: Record<MeshCatalogEntry["fit"], string> = {
  comfortable: "Fits well",
  tight: "Tight fit",
  tradeoff: "Trade-off",
  too_large: "Too large",
};

const FIT_CLASS: Record<MeshCatalogEntry["fit"], string> = {
  comfortable: "text-green-600 dark:text-green-400",
  tight: "text-amber-600 dark:text-amber-400",
  tradeoff: "text-orange-600 dark:text-orange-400",
  too_large: "text-destructive",
};

/**
 * Hardware-ranked curated model list (mesh-console's diagnose pattern).
 * Click a row to fill the model field. Models too large for this machine are
 * listed but disabled — honest about why, instead of hiding them.
 */
function CatalogPicker({
  catalog,
  disabled,
  onPick,
  selected,
}: {
  catalog: MeshModelCatalog;
  disabled: boolean;
  onPick: (name: string) => void;
  selected: string;
}) {
  const [expanded, setExpanded] = React.useState(false);
  // Above the fold: the Buzz-curated picks (models known to work well with
  // agents on shared compute). Below: everything else, as advanced options.
  const curated = catalog.entries.filter((e) => e.curated);
  const advanced = catalog.entries.filter((e) => !e.curated);
  const visible = expanded ? catalog.entries : curated;
  return (
    <div className="mt-1" data-testid="mesh-share-compute-catalog">
      <p className="text-sm font-normal text-muted-foreground">
        Recommended for this machine
        {catalog.gpuName ? ` (${catalog.gpuName}, ` : " ("}
        {catalog.vramDisplay} AI memory):
      </p>
      <ul className="mt-1.5 flex max-h-56 flex-col gap-1 overflow-y-auto">
        {visible.map((entry) => {
          const isSelected = entry.name === selected;
          const tooLarge = entry.fit === "too_large";
          return (
            <li key={entry.name}>
              <button
                className={cn(
                  "flex w-full items-baseline gap-2 rounded border px-2 py-1 text-left text-sm",
                  isSelected
                    ? "border-primary/60 bg-primary/10"
                    : "border-border/60 bg-muted/20 hover:bg-muted/40",
                  "disabled:cursor-not-allowed disabled:opacity-50",
                )}
                data-testid={`mesh-catalog-${entry.name}`}
                disabled={disabled || tooLarge}
                onClick={() => onPick(entry.name)}
                title={entry.description}
                type="button"
              >
                <span className="min-w-0 truncate font-medium">
                  {entry.name}
                </span>
                <span className="shrink-0 text-muted-foreground">
                  {entry.size}
                </span>
                <span className={cn("shrink-0", FIT_CLASS[entry.fit])}>
                  {FIT_LABEL[entry.fit]}
                </span>
                {entry.recommended ? (
                  <span className="shrink-0 rounded bg-primary/15 px-1.5 text-2xs font-medium text-primary">
                    Recommended
                  </span>
                ) : null}
                {entry.installed ? (
                  <span className="shrink-0 text-2xs text-muted-foreground">
                    Installed
                  </span>
                ) : null}
              </button>
            </li>
          );
        })}
      </ul>
      {advanced.length > 0 ? (
        <button
          className="mt-1 text-sm text-muted-foreground underline hover:text-foreground"
          data-testid="mesh-catalog-advanced-toggle"
          onClick={() => setExpanded((v) => !v)}
          type="button"
        >
          {expanded
            ? "Hide advanced models"
            : `Advanced: ${advanced.length} more models`}
        </button>
      ) : null}
    </div>
  );
}

function StatusLine({
  pendingAction,
  status,
}: {
  pendingAction: "start" | "stop" | null;
  status: MeshNodeStatus | null;
}) {
  if (pendingAction === "start") {
    return <p className="text-sm text-muted-foreground">Starting…</p>;
  }
  if (pendingAction === "stop") {
    return <p className="text-sm text-muted-foreground">Stopping…</p>;
  }
  if (!status) {
    return <p className="text-sm text-muted-foreground">Checking status…</p>;
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
        Sharing{modelLabel ? ` ${modelLabel}` : ""} with relay members.
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
