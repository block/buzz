import type { ImetaEntry } from "./types";

export type ParsedImetaEntry = ImetaEntry & {
  url: string;
  m: string;
  x: string;
  size: number;
  blurhash?: string;
  alt?: string;
};

export function parseImetaTags(
  tags: string[][],
): Map<string, ParsedImetaEntry> {
  const map = new Map<string, ParsedImetaEntry>();
  for (const tag of tags) {
    if (tag[0] !== "imeta") continue;
    const entry: Partial<ParsedImetaEntry> = {};
    for (const part of tag.slice(1)) {
      const spaceIdx = part.indexOf(" ");
      if (spaceIdx === -1) continue;
      const key = part.slice(0, spaceIdx);
      const val = part.slice(spaceIdx + 1);
      switch (key) {
        case "url":
          entry.url = val;
          break;
        case "m":
          entry.m = val;
          break;
        case "x":
          entry.x = val;
          break;
        case "size":
          entry.size = parseInt(val, 10);
          break;
        case "dim":
          entry.dim = val;
          break;
        case "blurhash":
          entry.blurhash = val;
          break;
        case "alt":
          entry.alt = val;
          break;
        case "thumb":
          entry.thumb = val;
          break;
        case "duration":
          entry.duration = parseFloat(val);
          break;
        case "image":
          entry.image = val;
          break;
        case "filename":
          entry.filename = val;
          break;
        case "waveform": {
          const samples = val
            .trim()
            .split(/\s+/)
            .map(Number)
            .filter(
              (sample) =>
                Number.isInteger(sample) && sample >= 0 && sample <= 100,
            )
            .slice(0, 100)
            .map((sample) => sample / 100);
          if (samples.length > 0) entry.waveform = samples;
          break;
        }
      }
    }
    if (entry.url) map.set(entry.url, entry as ParsedImetaEntry);
  }
  return map;
}
