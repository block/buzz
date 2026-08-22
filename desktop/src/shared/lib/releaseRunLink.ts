export type ReleaseRunTrack = {
  id: string;
  artist: string;
  title: string;
  version?: string;
  label?: string;
  releaseDate: string;
  artworkUrl?: string;
  source: string;
  sourceUrl?: string;
  detailsUrl?: string;
};

export type ReleaseRunPayload = {
  version: 1;
  runId: string;
  runName: string;
  status: string;
  checked: number;
  released: number;
  held: number;
  sourceHealth: string;
  finishedAt: string;
  tracks: ReleaseRunTrack[];
};

export type ReleaseRunViewState = "loading" | "empty" | "failed" | "ready";

const MAX_ENCODED_PAYLOAD_LENGTH = 48_000;
const MAX_TRACKS = 50;
const RUN_ID_PATTERN = /^[A-Za-z0-9._:-]{1,120}$/;
const BASE64URL_PATTERN = /^[A-Za-z0-9_-]+$/;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function boundedString(value: unknown, maximumLength: number): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > maximumLength) return null;
  return trimmed;
}

function optionalBoundedString(
  value: unknown,
  maximumLength: number,
): string | undefined | null {
  if (value == null || value === "") return undefined;
  return boundedString(value, maximumLength);
}

function count(value: unknown): number | null {
  return typeof value === "number" &&
    Number.isInteger(value) &&
    value >= 0 &&
    value <= 1_000_000
    ? value
    : null;
}

function httpsUrl(value: unknown): string | undefined | null {
  const candidate = optionalBoundedString(value, 2_000);
  if (candidate == null) return candidate;
  try {
    const parsed = new URL(candidate);
    return parsed.protocol === "https:" &&
      !parsed.username &&
      !parsed.password &&
      parsed.hostname
      ? parsed.href
      : null;
  } catch {
    return null;
  }
}

function decodeBase64Url(value: string): string | null {
  if (
    !value ||
    value.length > MAX_ENCODED_PAYLOAD_LENGTH ||
    !BASE64URL_PATTERN.test(value)
  ) {
    return null;
  }

  try {
    const padded = value.replace(/-/g, "+").replace(/_/g, "/");
    const padding = "=".repeat((4 - (padded.length % 4)) % 4);
    const binary = atob(`${padded}${padding}`);
    const bytes = Uint8Array.from(binary, (character) =>
      character.charCodeAt(0),
    );
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

function parseTrack(value: unknown): ReleaseRunTrack | null {
  if (!isRecord(value)) return null;

  const id = boundedString(value.id, 160);
  const artist = boundedString(value.artist, 200);
  const title = boundedString(value.title, 240);
  const version = optionalBoundedString(value.version, 160);
  const label = optionalBoundedString(value.label, 200);
  const releaseDate = boundedString(value.releaseDate, 40);
  const artworkUrl = httpsUrl(value.artworkUrl);
  const source = boundedString(value.source, 100);
  const sourceUrl = httpsUrl(value.sourceUrl);
  const detailsUrl = httpsUrl(value.detailsUrl);

  if (
    !id ||
    !artist ||
    !title ||
    !releaseDate ||
    !source ||
    version === null ||
    label === null ||
    artworkUrl === null ||
    sourceUrl === null ||
    detailsUrl === null
  ) {
    return null;
  }

  return {
    id,
    artist,
    title,
    ...(version ? { version } : {}),
    ...(label ? { label } : {}),
    releaseDate,
    ...(artworkUrl ? { artworkUrl } : {}),
    source,
    ...(sourceUrl ? { sourceUrl } : {}),
    ...(detailsUrl ? { detailsUrl } : {}),
  };
}

export function parseReleaseRunLink(href: string): ReleaseRunPayload | null {
  let parsed: URL;
  try {
    parsed = new URL(href);
  } catch {
    return null;
  }

  if (
    parsed.protocol !== "buzz:" ||
    parsed.hostname !== "release-run" ||
    (parsed.pathname !== "" && parsed.pathname !== "/") ||
    parsed.hash ||
    parsed.username ||
    parsed.password ||
    parsed.port
  ) {
    return null;
  }

  if (
    [...parsed.searchParams.keys()].some((key) => key !== "data") ||
    parsed.searchParams.getAll("data").length !== 1
  ) {
    return null;
  }

  const decoded = decodeBase64Url(parsed.searchParams.get("data") ?? "");
  if (!decoded) return null;

  let raw: unknown;
  try {
    raw = JSON.parse(decoded);
  } catch {
    return null;
  }
  if (!isRecord(raw) || raw.version !== 1) return null;

  const runId = boundedString(raw.runId, 120);
  const runName = boundedString(raw.runName, 200);
  const status = boundedString(raw.status, 80);
  const checked = count(raw.checked);
  const released = count(raw.released);
  const held = count(raw.held);
  const sourceHealth = boundedString(raw.sourceHealth, 500);
  const finishedAt = boundedString(raw.finishedAt, 80);

  if (
    !runId ||
    !RUN_ID_PATTERN.test(runId) ||
    !runName ||
    !status ||
    checked === null ||
    released === null ||
    held === null ||
    !sourceHealth ||
    !finishedAt ||
    Number.isNaN(Date.parse(finishedAt)) ||
    !Array.isArray(raw.tracks) ||
    raw.tracks.length > MAX_TRACKS
  ) {
    return null;
  }

  const tracks = raw.tracks.map(parseTrack);
  if (tracks.some((track) => track === null)) return null;
  if (released !== tracks.length) return null;

  return {
    version: 1,
    runId,
    runName,
    status,
    checked,
    released,
    held,
    sourceHealth,
    finishedAt,
    tracks: tracks as ReleaseRunTrack[],
  };
}

export function releaseRunViewState(
  payload: ReleaseRunPayload,
): ReleaseRunViewState {
  const status = payload.status.toLowerCase();
  if (/running|pending|queued|started/.test(status)) return "loading";
  if (/failed|error|critical/.test(status) && payload.tracks.length === 0) {
    return "failed";
  }
  return payload.tracks.length === 0 ? "empty" : "ready";
}

export function buildReleaseRunLink(payload: ReleaseRunPayload): string {
  const json = JSON.stringify(payload);
  const bytes = new TextEncoder().encode(json);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  const data = btoa(binary)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
  return `buzz://release-run?data=${data}`;
}
