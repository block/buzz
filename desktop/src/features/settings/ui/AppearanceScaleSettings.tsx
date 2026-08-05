import {
  AVATAR_SCALE_PRESETS,
  DEFAULT_AVATAR_SCALE,
  MAX_AVATAR_SCALE,
  MIN_AVATAR_SCALE,
  avatarScalePresetIndex,
  formatAvatarScalePercent,
  setAvatarScale,
} from "@/shared/lib/avatarScale";
import {
  CHAT_SCALE_PRESETS,
  DEFAULT_CHAT_SCALE,
  MAX_CHAT_SCALE,
  MIN_CHAT_SCALE,
  chatScalePresetIndex,
  formatChatScalePercent,
  setChatScale,
} from "@/shared/lib/chatScale";
import {
  DEFAULT_TEXT_SCALE,
  MAX_TEXT_SCALE,
  MIN_TEXT_SCALE,
  TEXT_SCALE_PRESETS,
  formatTextScalePercent,
  setTextScale,
  textScalePresetIndex,
} from "@/shared/lib/textScale";
import { useAvatarScale } from "@/shared/lib/useAvatarScale";
import { useChatScale } from "@/shared/lib/useChatScale";
import { useTextScale } from "@/shared/lib/useTextScale";
import { Button } from "@/shared/ui/button";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

type AppearanceScaleRowProps = {
  title: string;
  description: string;
  testIdPrefix: string;
  presets: readonly number[];
  minLabel: string;
  maxLabel: string;
  valueLabel: string;
  isDefault: boolean;
  presetIndex: number;
  onChange: (scale: number) => void;
  onReset: () => void;
};

function AppearanceScaleRow({
  title,
  description,
  testIdPrefix,
  presets,
  minLabel,
  maxLabel,
  valueLabel,
  isDefault,
  presetIndex,
  onChange,
  onReset,
}: AppearanceScaleRowProps) {
  return (
    <SettingsOptionRow className="flex-col items-stretch gap-3">
      <div className="min-w-0 flex-1 basis-full">
        <p className="text-sm font-medium">{title}</p>
        <p className="text-sm font-normal text-muted-foreground">
          {description}
        </p>
      </div>
      <div className="flex w-full min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <span
          aria-hidden="true"
          className="shrink-0 text-2xs tabular-nums text-muted-foreground"
        >
          {minLabel}
        </span>
        <div className="relative min-h-11 min-w-24 flex-1 basis-40">
          <input
            aria-label={title}
            aria-valuemax={presets.length - 1}
            aria-valuemin={0}
            aria-valuenow={presetIndex}
            aria-valuetext={valueLabel}
            className="absolute inset-y-0 left-0 right-0 m-auto h-11 w-full cursor-pointer appearance-none bg-transparent accent-primary [&::-moz-range-track]:h-1.5 [&::-moz-range-track]:rounded-full [&::-moz-range-track]:bg-foreground/15 [&::-webkit-slider-runnable-track]:h-1.5 [&::-webkit-slider-runnable-track]:rounded-full [&::-webkit-slider-runnable-track]:bg-foreground/15 [&::-webkit-slider-thumb]:-mt-1.5"
            data-testid={`${testIdPrefix}-slider`}
            max={presets.length - 1}
            min={0}
            onChange={(event) => {
              const index = Number(event.target.value);
              const next = presets[index];
              if (next != null) {
                onChange(next);
              }
            }}
            step={1}
            type="range"
            value={presetIndex}
          />
        </div>
        <span
          aria-hidden="true"
          className="shrink-0 text-2xs tabular-nums text-muted-foreground"
        >
          {maxLabel}
        </span>
        <span
          className="w-12 shrink-0 text-right text-sm tabular-nums font-medium text-foreground"
          data-testid={`${testIdPrefix}-value`}
        >
          {valueLabel}
        </span>
        <Button
          className="h-7 shrink-0 rounded-full border border-border/50 bg-muted/45 px-2.5 text-xs font-medium shadow-none hover:bg-muted/70 disabled:opacity-40"
          data-testid={`${testIdPrefix}-reset`}
          disabled={isDefault}
          onClick={onReset}
          size="sm"
          type="button"
          variant="ghost"
        >
          Reset
        </Button>
      </div>
    </SettingsOptionRow>
  );
}

/**
 * Appearance scale controls: global UI (Cmd/Ctrl +/-), chat text, and avatars.
 * All three share the 75%–500% ladder; chat/avatar are relative to interface.
 */
export function AppearanceScaleSettings() {
  const textScale = useTextScale();
  const chatScale = useChatScale();
  const avatarScale = useAvatarScale();

  return (
    <SettingsOptionGroup className="mb-6">
      <AppearanceScaleRow
        description="Enlarge or shrink the whole app UI. Keyboard: Cmd/Ctrl + / − / 0. Chat and avatar sizes are relative to this."
        isDefault={textScale === DEFAULT_TEXT_SCALE}
        maxLabel={formatTextScalePercent(MAX_TEXT_SCALE)}
        minLabel={formatTextScalePercent(MIN_TEXT_SCALE)}
        onChange={setTextScale}
        onReset={() => setTextScale(DEFAULT_TEXT_SCALE)}
        presetIndex={textScalePresetIndex(textScale)}
        presets={TEXT_SCALE_PRESETS}
        testIdPrefix="interface-scale"
        title="Interface scale"
        valueLabel={formatTextScalePercent(textScale)}
      />
      <AppearanceScaleRow
        description="Message body and author names in channels and threads. Relative to Interface scale."
        isDefault={chatScale === DEFAULT_CHAT_SCALE}
        maxLabel={formatChatScalePercent(MAX_CHAT_SCALE)}
        minLabel={formatChatScalePercent(MIN_CHAT_SCALE)}
        onChange={setChatScale}
        onReset={() => setChatScale(DEFAULT_CHAT_SCALE)}
        presetIndex={chatScalePresetIndex(chatScale)}
        presets={CHAT_SCALE_PRESETS}
        testIdPrefix="chat-scale"
        title="Chat text size"
        valueLabel={formatChatScalePercent(chatScale)}
      />
      <AppearanceScaleRow
        description="Identity avatars, thread rails, hover profile cards, and the profile panel hero. Relative to Interface scale."
        isDefault={avatarScale === DEFAULT_AVATAR_SCALE}
        maxLabel={formatAvatarScalePercent(MAX_AVATAR_SCALE)}
        minLabel={formatAvatarScalePercent(MIN_AVATAR_SCALE)}
        onChange={setAvatarScale}
        onReset={() => setAvatarScale(DEFAULT_AVATAR_SCALE)}
        presetIndex={avatarScalePresetIndex(avatarScale)}
        presets={AVATAR_SCALE_PRESETS}
        testIdPrefix="avatar-scale"
        title="Avatar & profile size"
        valueLabel={formatAvatarScalePercent(avatarScale)}
      />
    </SettingsOptionGroup>
  );
}
