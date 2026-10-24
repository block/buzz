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

/** Locale-aware relative time for display only. */
export function uiRelativeTime(
  value: number,
  unit: Intl.RelativeTimeFormatUnit,
): string {
  return new Intl.RelativeTimeFormat(
    i18n.resolvedLanguage === "ko" ? "ko-KR" : "en-US",
    {
      numeric: "always",
    },
  ).format(value, unit);
}
