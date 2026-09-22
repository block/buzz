import {
  setLanguagePreference,
  useLanguagePreference,
  type LanguagePreference,
} from "@/i18n/language";
import { useTranslation } from "@/i18n";
import { SegmentedControl } from "@/shared/ui/segmented-control";

import { SettingsOptionRow } from "./SettingsOptionGroup";

/**
 * Interface-language picker, top of the Settings → Appearance →
 * “Preferences” card.
 *
 * Switching applies immediately — no reload, no community remount — and
 * persists per device (FR-001 / FR-002). The three options map to the
 * `LanguagePreference` union; “System” defers to `navigator.languages`.
 */
export function LanguageSetting() {
  const { t } = useTranslation();
  const preference = useLanguagePreference();

  const options: readonly {
    value: LanguagePreference;
    label: string;
  }[] = [
    {
      value: "system",
      label: t("settings.appearance.language.option.system"),
    },
    { value: "en", label: t("settings.appearance.language.option.en") },
    {
      value: "zh-Hans",
      label: t("settings.appearance.language.option.zh-Hans"),
    },
  ];

  return (
    <SettingsOptionRow data-testid="language-row">
      <div className="min-w-0">
        <p className="text-sm font-medium">
          {t("settings.appearance.language.label")}
        </p>
        <p
          className="text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
        >
          {t("settings.appearance.language.description")}
        </p>
      </div>
      <SegmentedControl
        legend={t("settings.appearance.language.label")}
        onValueChange={setLanguagePreference}
        optionTestIdPrefix="language"
        options={options}
        size="default"
        testId="language-control"
        value={preference}
      />
    </SettingsOptionRow>
  );
}
