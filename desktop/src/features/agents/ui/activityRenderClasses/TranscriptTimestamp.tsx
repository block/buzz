import { formatTranscriptTime } from "../agentSessionUtils";

const fullDateTimeFormat = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  second: "2-digit",
});

export function TranscriptTimestamp({ timestamp }: { timestamp: string }) {
  const formatted = formatTranscriptTime(timestamp);
  if (!formatted) return null;
  const date = new Date(timestamp);
  const fullDateTime = Number.isNaN(date.getTime())
    ? timestamp
    : fullDateTimeFormat.format(date);
  return (
    <span
      className="shrink-0 cursor-default text-xs text-muted-foreground/60"
      title={fullDateTime}
    >
      {formatted}
    </span>
  );
}
