import { ChevronDown, MoreVertical } from "lucide-react";
import * as React from "react";

import type { VoiceRegistryEntry } from "@/features/settings/ui/voiceSettingsLogic";
import {
  voiceOptionLabel,
  voicesForBackend,
} from "@/features/settings/ui/voiceSettingsLogic";
import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Separator } from "@/shared/ui/separator";
import { Switch } from "@/shared/ui/switch";

export type HuddleAgentVoiceSettings = {
  enabled: boolean;
  voice_key: string;
};

type AgentVoiceMenuProps = {
  agentPubkey: string;
  displayName: string;
  registry: VoiceRegistryEntry[];
  settings: HuddleAgentVoiceSettings | undefined;
  onSettingsChange: (settings: HuddleAgentVoiceSettings) => void;
};

export function AgentVoiceMenu({
  agentPubkey,
  displayName,
  registry,
  settings,
  onSettingsChange,
}: AgentVoiceMenuProps) {
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const voices = voicesForBackend(registry, "pocket");
  const selectedVoice =
    voices.find((voice) => voice.key === settings?.voice_key) ?? voices[0];

  const update = React.useCallback(
    async (
      command: "set_huddle_agent_tts_enabled" | "set_huddle_agent_voice",
      payload: Record<string, unknown>,
    ) => {
      setBusy(true);
      setError(null);
      try {
        const next = await invokeTauri<HuddleAgentVoiceSettings>(command, {
          agentPubkey,
          ...payload,
        });
        onSettingsChange(next);
      } catch (updateError) {
        setError(
          updateError instanceof Error
            ? updateError.message
            : "Agent voice could not be updated.",
        );
      } finally {
        setBusy(false);
      }
    },
    [agentPubkey, onSettingsChange],
  );

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          aria-label={`Voice settings for ${displayName}`}
          className="absolute right-1.5 top-1.5 z-20 h-7 w-7 rounded-md bg-transparent text-muted-foreground shadow-none hover:bg-muted/70 hover:text-foreground"
          data-testid="huddle-agent-voice-menu-trigger"
          size="icon"
          type="button"
          variant="ghost"
        >
          <MoreVertical className="h-4 w-4" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-64 space-y-3 p-3" sideOffset={8}>
        <div className="flex items-center justify-between gap-3">
          <label
            className="text-sm font-medium"
            htmlFor={`agent-tts-${agentPubkey}`}
          >
            Agent text-to-speech
          </label>
          <Switch
            checked={settings?.enabled ?? true}
            data-testid="huddle-agent-tts-toggle"
            disabled={!settings || busy}
            id={`agent-tts-${agentPubkey}`}
            onCheckedChange={(enabled) => {
              void update("set_huddle_agent_tts_enabled", { enabled });
            }}
          />
        </div>

        {settings?.enabled ? (
          <>
            <Separator />
            <div className="space-y-2">
              <span className="text-sm font-medium">Agent voice</span>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    className="w-full justify-between"
                    data-testid="huddle-agent-voice-selector"
                    disabled={busy || voices.length === 0}
                    variant="outline"
                  >
                    {selectedVoice
                      ? voiceOptionLabel(selectedVoice, voices)
                      : "Unavailable"}
                    <ChevronDown className="h-4 w-4" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent
                  align="end"
                  className="max-h-72 overflow-y-auto"
                >
                  <DropdownMenuRadioGroup
                    onValueChange={(voiceKey) => {
                      void update("set_huddle_agent_voice", { voiceKey });
                    }}
                    value={selectedVoice?.key}
                  >
                    {voices.map((voice) => (
                      <DropdownMenuRadioItem key={voice.key} value={voice.key}>
                        {voiceOptionLabel(voice, voices)}
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          </>
        ) : null}

        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </PopoverContent>
    </Popover>
  );
}
