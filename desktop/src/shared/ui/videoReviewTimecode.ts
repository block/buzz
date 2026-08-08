export type VideoReviewTimecode = {
  seconds: number;
  text: string;
  timecode: string;
};

const TIMECODE_RE =
  /^\s*\[((?:(?:\d{1,2}:)?\d{1,2}:)?\d{2}(?:\.\d{1,3})?)\]\s*/;

function parseTimecode(value: string): number | null {
  const parts = value.split(":").map((part) => Number(part));
  if (parts.some((part) => !Number.isFinite(part) || part < 0)) {
    return null;
  }

  if (parts.length === 2) {
    return parts[0] * 60 + parts[1];
  }

  if (parts.length === 3) {
    return parts[0] * 3600 + parts[1] * 60 + parts[2];
  }

  return null;
}

export function parseVideoReviewTimecode(
  content: string,
): VideoReviewTimecode | null {
  const match = content.match(TIMECODE_RE);
  if (!match) return null;

  const seconds = parseTimecode(match[1]);
  if (seconds === null) return null;

  return {
    seconds,
    text: content.slice(match[0].length).trim(),
    timecode: match[1],
  };
}
