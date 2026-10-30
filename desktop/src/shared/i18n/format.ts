import { i18n } from "@/i18n";

/** Use the selected UI locale; persisted timestamps and time zones stay unchanged. */
export function uiDateFormatter(options: Intl.DateTimeFormatOptions) {
  const english = new Intl.DateTimeFormat("en-US", options);
  const korean = new Intl.DateTimeFormat("ko-KR", options);
  return {
    format(value: Date | number): string {
      return (i18n.resolvedLanguage === "ko" ? korean : english).format(value);
    },
  };
}

/** Reusable relative-time formatting that follows the current UI language. */
export function uiRelativeTimeFormatter(
  options: Intl.RelativeTimeFormatOptions,
) {
  const english = new Intl.RelativeTimeFormat("en-US", options);
  const korean = new Intl.RelativeTimeFormat("ko-KR", options);
  return {
    format(value: number, unit: Intl.RelativeTimeFormatUnit): string {
      return (i18n.resolvedLanguage === "ko" ? korean : english).format(
        value,
        unit,
      );
    },
  };
}

const relativeTime = uiRelativeTimeFormatter({ numeric: "always" });

/** Locale-aware relative time for display only. */
export function uiRelativeTime(
  value: number,
  unit: Intl.RelativeTimeFormatUnit,
): string {
  return relativeTime.format(value, unit);
}
