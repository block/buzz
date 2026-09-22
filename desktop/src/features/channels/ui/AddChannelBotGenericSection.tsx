import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { useTranslation } from "@/i18n";

type AddChannelBotGenericSectionProps = {
  disabled: boolean;
  name: string;
  prompt: string;
  onNameChange: (value: string) => void;
  onPromptChange: (value: string) => void;
};

export function AddChannelBotGenericSection({
  disabled,
  name,
  prompt,
  onNameChange,
  onPromptChange,
}: AddChannelBotGenericSectionProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-5 rounded-2xl border border-border/70 bg-card/70 p-4">
      <div>
        <div className="text-sm font-medium">
          {t("channels.bot.generic-agent")}
        </div>
        <p className="text-xs text-muted-foreground">
          {t("channels.bot.generic-description")}
        </p>
      </div>

      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="channel-generic-name">
          {t("channels.bot.name")}
        </label>
        <Input
          autoCapitalize="none"
          autoCorrect="off"
          disabled={disabled}
          id="channel-generic-name"
          onChange={(event) => onNameChange(event.target.value)}
          spellCheck={false}
          value={name}
        />
        <p className="text-xs text-muted-foreground">
          {t("channels.bot.name-hint")}
        </p>
      </div>

      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="channel-generic-prompt">
          {t("channels.bot.prompt")}
        </label>
        <Textarea
          className="min-h-24"
          disabled={disabled}
          id="channel-generic-prompt"
          onChange={(event) => onPromptChange(event.target.value)}
          placeholder={t("channels.bot.prompt-placeholder")}
          value={prompt}
        />
        <p className="text-xs text-muted-foreground">
          {t("channels.bot.prompt-hint")}
        </p>
      </div>
    </div>
  );
}
