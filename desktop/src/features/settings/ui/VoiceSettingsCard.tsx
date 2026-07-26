import { useEffect, useState } from "react";
import { Check, Download, LoaderCircle } from "lucide-react";
import {
  downloadSiriTtsVoice,
  getTtsSettings,
  listSiriTtsVoices,
  setTtsSettings,
  type SiriTtsVoice,
  type TtsBackend,
  type TtsSettings,
} from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

const DEFAULT_SETTINGS: TtsSettings = {
  backend: "pocket",
  siri_voice: null,
  siri_language: null,
  siri_rate: 1,
};

const LANGUAGE_PREFIX = navigator.language.split(/[-_]/)[0] || "en";
const SIRI_RATES = [0.75, 1, 1.25, 1.5] as const;

function formatSize(bytes: number): string {
  if (bytes <= 0) return "";
  return `${Math.max(1, Math.round(bytes / 1_000_000))} MB`;
}

export function VoiceSettingsCard() {
  const [settings, setSettings] = useState<TtsSettings>(DEFAULT_SETTINGS);
  const [voices, setVoices] = useState<SiriTtsVoice[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [downloading, setDownloading] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refreshVoices = async () => {
    const discovered = await listSiriTtsVoices(LANGUAGE_PREFIX);
    setVoices(discovered);
    return discovered;
  };

  useEffect(() => {
    let cancelled = false;
    void Promise.all([getTtsSettings(), listSiriTtsVoices(LANGUAGE_PREFIX)])
      .then(([stored, discovered]) => {
        if (cancelled) return;
        setSettings(stored);
        setVoices(discovered);
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(String(reason));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const persist = async (next: TtsSettings) => {
    setSaving(true);
    setError(null);
    try {
      await setTtsSettings(next);
      setSettings(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  const selectBackend = async (backend: TtsBackend) => {
    if (backend === "siri" && !settings.siri_voice) {
      const installed = voices.find(
        (voice) => voice.availability === "installed",
      );
      if (!installed) {
        setError("Download a Siri voice before selecting the Siri backend.");
        return;
      }
      await persist({
        ...settings,
        backend,
        siri_voice: installed.name,
        siri_language: installed.language,
      });
      return;
    }
    await persist({ ...settings, backend });
  };

  const downloadVoice = async (voice: SiriTtsVoice) => {
    const key = `${voice.name}|${voice.language}`;
    setDownloading(key);
    setError(null);
    try {
      const installed = await downloadSiriTtsVoice(voice.name, voice.language);
      const discovered = await refreshVoices();
      if (!settings.siri_voice) {
        const resolved =
          discovered.find(
            (candidate) =>
              candidate.name === installed.name &&
              candidate.language === installed.language,
          ) ?? installed;
        await persist({
          ...settings,
          siri_voice: resolved.name,
          siri_language: resolved.language,
        });
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setDownloading(null);
    }
  };

  return (
    <section className="min-w-0" data-testid="settings-voice">
      <SettingsSectionHeader
        title="Voice"
        description="Choose how Buzz speaks agent messages during huddles."
      />

      <SettingsOptionGroup>
        <SettingsOptionRow>
          <div className="min-w-0">
            <p className="text-sm font-medium">Speech engine</p>
            <p className="text-sm text-muted-foreground">
              Pocket TTS runs offline. Siri TTS streams from macOS and supports
              downloaded system voices.
            </p>
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button disabled={loading || saving} variant="outline">
                {settings.backend === "pocket" ? "Pocket TTS" : "Siri TTS"}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuRadioGroup
                onValueChange={(value) =>
                  void selectBackend(value as TtsBackend)
                }
                value={settings.backend}
              >
                <DropdownMenuRadioItem value="pocket">
                  Pocket TTS
                </DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="siri">
                  Siri TTS (Experimental)
                </DropdownMenuRadioItem>
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </SettingsOptionRow>

        {settings.backend === "siri" && (
          <SettingsOptionRow className="items-start border-b border-border/60">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">Siri voice</p>
              <p className="mb-3 text-sm text-muted-foreground">
                Installed voices can be selected immediately. Other voices are
                downloaded by macOS before Buzz enables them.
              </p>
              <div className="grid gap-2">
                {voices.map((voice) => {
                  const key = `${voice.name}|${voice.language}`;
                  const selected =
                    settings.siri_voice === voice.name &&
                    settings.siri_language === voice.language;
                  const isDownloading = downloading === key;
                  return (
                    <button
                      className="flex w-full items-center justify-between rounded-xl border border-border/70 px-3 py-2 text-left text-sm hover:bg-muted/40 disabled:cursor-wait disabled:opacity-60"
                      disabled={saving || downloading !== null}
                      key={key}
                      onClick={() => {
                        if (voice.availability === "installed") {
                          void persist({
                            ...settings,
                            siri_voice: voice.name,
                            siri_language: voice.language,
                          });
                        } else {
                          void downloadVoice(voice);
                        }
                      }}
                      type="button"
                    >
                      <span>
                        <span className="block font-medium">{voice.name}</span>
                        <span className="text-muted-foreground">
                          {voice.language}
                          {voice.size_bytes > 0
                            ? ` · ${formatSize(voice.size_bytes)}`
                            : ""}
                        </span>
                      </span>
                      {selected ? (
                        <Check className="h-4 w-4" />
                      ) : isDownloading ? (
                        <LoaderCircle className="h-4 w-4 animate-spin" />
                      ) : voice.availability === "available" ? (
                        <Download className="h-4 w-4 text-muted-foreground" />
                      ) : null}
                    </button>
                  );
                })}
                {!loading && voices.length === 0 && (
                  <p className="text-sm text-muted-foreground">
                    No downloadable Siri voices were found for the current
                    language.
                  </p>
                )}
              </div>
            </div>
          </SettingsOptionRow>
        )}

        {settings.backend === "siri" && (
          <SettingsOptionRow>
            <div className="min-w-0">
              <p className="text-sm font-medium">Speech rate</p>
              <p className="text-sm text-muted-foreground">
                Siri adjusts timing while preserving the selected voice&apos;s
                pitch.
              </p>
            </div>
            <div className="flex items-center gap-1">
              {SIRI_RATES.map((rate) => (
                <Button
                  disabled={saving}
                  key={rate}
                  onClick={() => void persist({ ...settings, siri_rate: rate })}
                  size="sm"
                  variant={settings.siri_rate === rate ? "default" : "outline"}
                >
                  {rate}×
                </Button>
              ))}
            </div>
          </SettingsOptionRow>
        )}
      </SettingsOptionGroup>

      {error && (
        <p className="mt-3 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}
