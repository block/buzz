import * as React from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Play, Trash2, Upload, Volume2 } from "lucide-react";

import { invokeTauri } from "@/shared/api/tauri";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
import {
  selectedVoiceForBackend,
  type VoiceRegistryEntry,
  voiceOptionLabel,
  voicesForBackend,
} from "./voiceSettingsLogic";

export type TtsSettings = {
  version: number;
  agentTextToSpeech: boolean;
  voicePreferences: string[];
  speechLanguage: "pt-BR" | "en-US";
};

type TtsVoiceMutation = {
  settings: TtsSettings;
  registry: VoiceRegistryEntry[];
};

export function VoiceSettingsCard() {
  const { t } = useTranslation();
  const [settings, setSettings] = React.useState<TtsSettings | null>(null);
  const [registry, setRegistry] = React.useState<VoiceRegistryEntry[]>([]);
  const [busy, setBusy] = React.useState(false);
  const [previewing, setPreviewing] = React.useState(false);
  const [deleteCandidate, setDeleteCandidate] =
    React.useState<VoiceRegistryEntry | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let disposed = false;
    Promise.all([
      invokeTauri<TtsSettings>("get_tts_settings"),
      invokeTauri<VoiceRegistryEntry[]>("list_voice_registry"),
    ])
      .then(([nextSettings, nextRegistry]) => {
        if (!disposed) {
          setSettings(nextSettings);
          setRegistry(nextRegistry);
        }
      })
      .catch((loadError) => {
        if (!disposed) {
          setError(
            loadError instanceof Error
              ? loadError.message
              : "Voice settings could not be loaded.",
          );
        }
      });
    return () => {
      disposed = true;
    };
  }, []);

  const saveEnabled = React.useCallback(async (enabled: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const saved = await invokeTauri<TtsSettings>("set_tts_enabled", {
        enabled,
      });
      setSettings(saved);
    } catch (saveError) {
      try {
        const state = await invokeTauri<{ tts_enabled: boolean }>(
          "get_huddle_state",
        );
        setSettings((current) =>
          current
            ? { ...current, agentTextToSpeech: state.tts_enabled }
            : current,
        );
      } catch {
        // Keep the last confirmed state when native reconciliation is
        // unavailable; the visible save error makes the failure explicit.
      }
      setError(
        saveError instanceof Error
          ? saveError.message
          : "Voice settings could not be saved.",
      );
    } finally {
      setBusy(false);
    }
  }, []);

  const savePocketVoice = React.useCallback(async (voiceKey: string) => {
    setBusy(true);
    setError(null);
    try {
      const saved = await invokeTauri<TtsSettings>("set_pocket_voice", {
        voiceKey,
      });
      setSettings(saved);
    } catch (saveError) {
      setError(
        saveError instanceof Error
          ? saveError.message
          : "Voice settings could not be saved.",
      );
    } finally {
      setBusy(false);
    }
  }, []);

  const saveSpeechLanguage = React.useCallback(
    async (language: "pt-BR" | "en-US") => {
      setBusy(true);
      setError(null);
      try {
        const saved = await invokeTauri<TtsSettings>("set_speech_language", {
          language,
        });
        setSettings(saved);
      } catch (saveError) {
        setError(
          saveError instanceof Error ? saveError.message : t("voice.saveError"),
        );
      } finally {
        setBusy(false);
      }
    },
    [t],
  );

  const importPocketVoice = React.useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await invokeTauri<TtsVoiceMutation | null>(
        "import_pocket_voice",
      );
      if (result) {
        setSettings(result.settings);
        setRegistry(result.registry);
      }
    } catch (importError) {
      setError(
        importError instanceof Error
          ? importError.message
          : "Voice could not be imported.",
      );
    } finally {
      setBusy(false);
    }
  }, []);

  const deletePocketVoice = React.useCallback(async (voiceKey: string) => {
    setBusy(true);
    setError(null);
    try {
      const result = await invokeTauri<TtsVoiceMutation>(
        "delete_pocket_voice",
        { voiceKey },
      );
      setSettings(result.settings);
      setRegistry(result.registry);
      setDeleteCandidate(null);
    } catch (deleteError) {
      setError(
        deleteError instanceof Error
          ? deleteError.message
          : "Voice could not be deleted.",
      );
    } finally {
      setBusy(false);
    }
  }, []);

  const voices = voicesForBackend(registry, "pocket");
  const selectedVoice = selectedVoiceForBackend(
    settings?.voicePreferences ?? [],
    voices,
  );
  const enabled = settings?.agentTextToSpeech ?? true;
  const controlsDisabled = !settings || busy || !enabled;

  return (
    <section className="min-w-0" data-testid="settings-voice">
      <SettingsSectionHeader
        title={t("voice.title")}
        description={t("voice.description")}
      />

      <div className="flex flex-col gap-4">
        <SettingsOptionGroup>
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">{t("voice.language")}</p>
              <p className="text-sm text-muted-foreground">
                {t("voice.languageDescription")}
              </p>
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  className="min-w-40 justify-between"
                  data-testid="speech-language-selector"
                  disabled={!settings || busy}
                  variant="outline"
                >
                  {settings?.speechLanguage === "pt-BR"
                    ? "Português (Brasil)"
                    : "English (US)"}
                  <ChevronDown className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuRadioGroup
                  onValueChange={(language) =>
                    void saveSpeechLanguage(language as "pt-BR" | "en-US")
                  }
                  value={settings?.speechLanguage}
                >
                  <DropdownMenuRadioItem value="pt-BR">
                    Português (Brasil)
                  </DropdownMenuRadioItem>
                  <DropdownMenuRadioItem value="en-US">
                    English (US)
                  </DropdownMenuRadioItem>
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </SettingsOptionRow>
        </SettingsOptionGroup>

        <SettingsOptionGroup>
          <SettingsOptionRow>
            <div className="min-w-0">
              <label
                className="text-sm font-medium"
                htmlFor="agent-text-to-speech-switch"
              >
                {t("voice.agentTts")}
              </label>
              <p className="text-sm text-muted-foreground">
                {t("voice.agentTtsDescription")}
              </p>
            </div>
            <Switch
              checked={enabled}
              data-testid="agent-text-to-speech-toggle"
              disabled={!settings || busy}
              id="agent-text-to-speech-switch"
              onCheckedChange={(checked) => {
                if (settings) void saveEnabled(checked);
              }}
            />
          </SettingsOptionRow>
        </SettingsOptionGroup>

        {settings?.speechLanguage === "pt-BR" ? (
          <SettingsOptionGroup>
            <SettingsOptionRow>
              <div className="min-w-0">
                <p className="text-sm font-medium">Piper — Faber</p>
                <p className="text-sm text-muted-foreground">
                  {t("voice.portugueseLocal")}
                </p>
              </div>
              <span className="text-sm text-muted-foreground">pt-BR</span>
            </SettingsOptionRow>
          </SettingsOptionGroup>
        ) : (
          <div
            aria-disabled={!enabled}
            className={cn(
              "transition-opacity",
              !enabled && "pointer-events-none opacity-45",
            )}
            data-testid="pocket-voice-controls"
          >
            <SettingsOptionGroup>
              <SettingsOptionRow>
                <div className="min-w-0">
                  <p className="text-sm font-medium">{t("voice.pocket")}</p>
                  <p className="text-sm text-muted-foreground">
                    {t("voice.localOnly")}
                  </p>
                </div>

                <div className="flex shrink-0 items-center gap-2">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        aria-label={`Pocket TTS voice: ${selectedVoice?.displayName ?? "Mary"}`}
                        className="min-w-32 justify-between"
                        data-testid="pocket-voice-selector"
                        disabled={controlsDisabled}
                        variant="outline"
                      >
                        {selectedVoice
                          ? voiceOptionLabel(selectedVoice, voices)
                          : "Mary"}
                        <ChevronDown className="h-4 w-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent
                      align="end"
                      className="max-h-80 overflow-y-auto"
                    >
                      <DropdownMenuRadioGroup
                        onValueChange={(voiceKey) => {
                          if (settings) void savePocketVoice(voiceKey);
                        }}
                        value={selectedVoice?.key}
                      >
                        {voices.map((voice) => (
                          <DropdownMenuRadioItem
                            key={voice.key}
                            value={voice.key}
                          >
                            {voiceOptionLabel(voice, voices)}
                          </DropdownMenuRadioItem>
                        ))}
                      </DropdownMenuRadioGroup>
                    </DropdownMenuContent>
                  </DropdownMenu>
                  <Button
                    aria-label={`Preview ${selectedVoice?.displayName ?? "Mary"}`}
                    data-testid="pocket-voice-preview"
                    disabled={controlsDisabled || previewing || !selectedVoice}
                    onClick={() => {
                      if (!selectedVoice) return;
                      setPreviewing(true);
                      setError(null);
                      void invokeTauri<void>("preview_pocket_voice", {
                        voiceKey: selectedVoice.key,
                      })
                        .catch((previewError) => {
                          setError(
                            previewError instanceof Error
                              ? previewError.message
                              : "Voice preview could not be played.",
                          );
                        })
                        .finally(() => setPreviewing(false));
                    }}
                    size="sm"
                    variant="outline"
                  >
                    {previewing ? (
                      <Volume2 className="h-4 w-4 animate-pulse" />
                    ) : (
                      <Play className="h-4 w-4" />
                    )}
                    {t("voice.preview")}
                  </Button>
                  <Button
                    data-testid="pocket-voice-import"
                    disabled={controlsDisabled}
                    onClick={() => void importPocketVoice()}
                    size="sm"
                    variant="outline"
                  >
                    <Upload className="h-4 w-4" />
                    {t("voice.add")}
                  </Button>
                  {selectedVoice?.key.startsWith("pocket:imported:") && (
                    <Button
                      aria-label={`Delete ${selectedVoice.displayName}`}
                      data-testid="pocket-voice-delete"
                      disabled={controlsDisabled}
                      onClick={() => setDeleteCandidate(selectedVoice)}
                      size="icon"
                      variant="ghost"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  )}
                </div>
              </SettingsOptionRow>
            </SettingsOptionGroup>
          </div>
        )}

        {error && (
          <p
            className="text-sm text-destructive"
            data-testid="voice-settings-error"
            role="alert"
          >
            {error}
          </p>
        )}
      </div>
      <AlertDialog
        onOpenChange={(open) => {
          if (!open) setDeleteCandidate(null);
        }}
        open={deleteCandidate !== null}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("voice.deleteConfirm")}</AlertDialogTitle>
            <AlertDialogDescription>
              {deleteCandidate
                ? t("voice.deleteDescription", {
                    name: deleteCandidate.displayName,
                  })
                : t("voice.deleteDescription", { name: t("voice.pocket") })}
              {selectedVoice?.key === deleteCandidate?.key &&
                " Mary will be selected instead."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>
              {t("common.cancel")}
            </AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              data-testid="confirm-pocket-voice-delete"
              disabled={busy || !deleteCandidate}
              onClick={(event) => {
                event.preventDefault();
                if (deleteCandidate) {
                  void deletePocketVoice(deleteCandidate.key);
                }
              }}
            >
              {t("voice.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
