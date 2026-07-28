import { cn } from "@/shared/lib/cn";
import { useI18n } from "./I18nProvider";
import type { Lang } from "./messages";

type LanguageToggleProps = {
  className?: string;
  /** compact: top chrome EN|中文; full: settings card radios */
  variant?: "compact" | "full";
  testId?: string;
};

const OPTIONS: Array<{ value: Lang; compactKey: "chrome.lang.en" | "chrome.lang.zh"; fullKey: "appearance.language.en" | "appearance.language.zh" }> =
  [
    { value: "zh", compactKey: "chrome.lang.zh", fullKey: "appearance.language.zh" },
    { value: "en", compactKey: "chrome.lang.en", fullKey: "appearance.language.en" },
  ];

export function LanguageToggle({
  className,
  variant = "compact",
  testId = "language-toggle",
}: LanguageToggleProps) {
  const { lang, setLang, t } = useI18n();

  if (variant === "full") {
    return (
      <div
        className={cn("flex flex-wrap gap-2", className)}
        data-testid={testId}
        role="group"
        aria-label={t("chrome.language")}
      >
        {OPTIONS.map((opt) => (
          <button
            key={opt.value}
            type="button"
            aria-pressed={lang === opt.value}
            data-testid={`language-option-${opt.value}`}
            className={cn(
              "rounded-lg border px-4 py-2 text-sm font-medium transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
              lang === opt.value
                ? "border-primary bg-primary/15 text-foreground"
                : "border-border text-muted-foreground hover:border-foreground/40 hover:text-foreground",
            )}
            onClick={() => setLang(opt.value)}
          >
            {t(opt.fullKey)}
          </button>
        ))}
      </div>
    );
  }

  return (
    <div
      className={cn(
        "inline-flex items-stretch overflow-hidden rounded-lg border border-sidebar-border bg-sidebar-accent/40 text-2xs font-semibold tracking-wide",
        className,
      )}
      data-testid={testId}
      role="group"
      aria-label={t("chrome.language")}
    >
      {OPTIONS.map((opt) => (
        <button
          key={opt.value}
          type="button"
          aria-pressed={lang === opt.value}
          data-testid={`language-option-${opt.value}`}
          className={cn(
            "px-2 py-1 transition-colors focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
            lang === opt.value
              ? "bg-primary text-primary-foreground"
              : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
          )}
          onClick={() => setLang(opt.value)}
        >
          {t(opt.compactKey)}
        </button>
      ))}
    </div>
  );
}
