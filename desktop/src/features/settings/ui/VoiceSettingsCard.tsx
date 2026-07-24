import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import { Check } from "lucide-react";

import {
  setDictationModelPreference,
  useDictationModelPreference,
} from "@/features/messages/lib/dictationModelPreference";
import { cn } from "@/shared/lib/cn";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

/** Mirror of the Rust `ModelStatus` serde encoding (models.rs). */
type ModelStatus =
  | "not_downloaded"
  | "ready"
  | { downloading: { progress_percent: number } }
  | { error: string };

/** Mirror of the Rust `SttModelInfo` (models.rs). */
type SttModelInfo = {
  id: string;
  languages: string;
  multilingual: boolean;
  selected: boolean;
  status: ModelStatus;
};

/**
 * Display copy per registry model id. Unknown ids (a future registry
 * addition) fall back to the id itself so the picker never hides a model.
 */
const MODEL_COPY: Record<
  string,
  { label: string; description: string; size: string }
> = {
  "parakeet-en": {
    label: "Standard",
    description: "Fast, English only. Light on punctuation.",
    size: "~120 MB",
  },
  "parakeet-v3": {
    label: "Enhanced",
    description:
      "Best accuracy and punctuation. 25 European languages, auto-detected.",
    size: "~465 MB",
  },
};

function isDownloading(
  status: ModelStatus,
): status is { downloading: { progress_percent: number } } {
  return typeof status === "object" && "downloading" in status;
}

function statusText(info: SttModelInfo): string {
  if (info.status === "ready") return "Downloaded";
  if (isDownloading(info.status)) {
    return `Downloading… ${info.status.downloading.progress_percent}%`;
  }
  const size = MODEL_COPY[info.id]?.size;
  if (typeof info.status === "object" && "error" in info.status) {
    return "Download failed — select to retry";
  }
  return size ? `${size} download` : "Not downloaded";
}

export function VoiceSettingsCard() {
  const preference = useDictationModelPreference();
  const [models, setModels] = React.useState<SttModelInfo[] | null>(null);
  const [loadError, setLoadError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setModels(await invoke<SttModelInfo[]>("get_dictation_models"));
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  // Poll for download progress only while a download is running.
  const downloading = models?.some((m) => isDownloading(m.status)) ?? false;
  React.useEffect(() => {
    if (!downloading) return;
    const timer = window.setInterval(() => void refresh(), 1000);
    return () => window.clearInterval(timer);
  }, [downloading, refresh]);

  // With no explicit preference, dictation uses the startup-selected model.
  const activeId = preference ?? models?.find((m) => m.selected)?.id ?? null;
  const activeModel = models?.find((m) => m.id === activeId);
  const fallbackModel = models?.find((m) => m.selected);

  const handleSelect = (info: SttModelInfo) => {
    setDictationModelPreference(info.id);
    if (info.status !== "ready") {
      void invoke("download_dictation_model", { id: info.id })
        .then(refresh)
        .catch((e) => setLoadError(e instanceof Error ? e.message : String(e)));
    }
  };

  return (
    <section className="min-w-0" data-testid="settings-voice">
      <SettingsSectionHeader
        title="Voice & dictation"
        description="Dictate messages in the composer: hold Space (or press ⌃Space) to talk, release to stop, press Enter to send. Speech-to-text runs entirely on this device. Audio never leaves your machine."
      />

      <h2 className="mb-2 text-lg font-semibold tracking-tight">
        Dictation model
      </h2>
      <SettingsOptionGroup>
        {(models ?? []).map((info) => {
          const copy = MODEL_COPY[info.id] ?? {
            label: info.id,
            description: info.languages,
            size: "",
          };
          const active = info.id === activeId;
          return (
            <SettingsOptionRow className="p-0" key={info.id}>
              <button
                aria-pressed={active}
                className="flex min-h-16 w-full items-center justify-between gap-4 px-4 py-3 text-left hover:bg-muted/40"
                data-testid={`dictation-model-${info.id}`}
                onClick={() => handleSelect(info)}
                type="button"
              >
                <div className="min-w-0">
                  <p className="text-sm font-medium">{copy.label}</p>
                  <p className="text-sm font-normal text-muted-foreground">
                    {copy.description}
                  </p>
                  <p className="mt-0.5 text-xs text-muted-foreground/80">
                    {statusText(info)}
                  </p>
                  {isDownloading(info.status) ? (
                    <div className="mt-1.5 h-1 w-48 overflow-hidden rounded-full bg-muted">
                      <div
                        className="h-full rounded-full bg-primary transition-[width]"
                        style={{
                          width: `${info.status.downloading.progress_percent}%`,
                        }}
                      />
                    </div>
                  ) : null}
                </div>
                <span
                  className={cn(
                    "flex h-5 w-5 shrink-0 items-center justify-center rounded-full border",
                    active
                      ? "border-primary bg-primary text-primary-foreground"
                      : "border-border/70",
                  )}
                >
                  {active ? <Check className="h-3.5 w-3.5" /> : null}
                </span>
              </button>
            </SettingsOptionRow>
          );
        })}
      </SettingsOptionGroup>

      {activeModel && activeModel.status !== "ready" && fallbackModel ? (
        <p className="mt-3 text-sm text-muted-foreground">
          {MODEL_COPY[fallbackModel.id]?.label ?? fallbackModel.id} is used
          until the download finishes.
        </p>
      ) : null}
      {loadError ? (
        <p className="mt-3 text-sm text-destructive">{loadError}</p>
      ) : null}
    </section>
  );
}
