/**
 * Spec for ```map fenced blocks. Authored by people or agents (for example
 * via `buzz canvas set`) as JSON; coordinates are [lng, lat] like GeoJSON.
 */
export type LngLat = [number, number];

export type MapMarker = {
  lng: number;
  lat: number;
  label?: string;
  color?: string;
};

export type MapLine = {
  coordinates: LngLat[];
  label?: string;
  color?: string;
};

export type MapSpec = {
  center?: LngLat;
  zoom?: number;
  height?: number;
  markers: MapMarker[];
  lines: MapLine[];
};

export type MapSpecResult =
  | { ok: true; spec: MapSpec }
  | { ok: false; error: string };

const MIN_HEIGHT = 160;
const MAX_HEIGHT = 800;

function isLngLat(value: unknown): value is LngLat {
  return (
    Array.isArray(value) &&
    value.length === 2 &&
    Number.isFinite(value[0]) &&
    Number.isFinite(value[1]) &&
    Math.abs(value[0]) <= 180 &&
    Math.abs(value[1]) <= 90
  );
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

export function parseMapSpec(source: string): MapSpecResult {
  let raw: unknown;
  try {
    raw = JSON.parse(source);
  } catch {
    return { ok: false, error: "Map block is not valid JSON." };
  }
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    return { ok: false, error: "Map block must be a JSON object." };
  }
  const input = raw as Record<string, unknown>;

  const markers: MapMarker[] = [];
  for (const [index, item] of (Array.isArray(input.markers)
    ? input.markers
    : []
  ).entries()) {
    const marker = (item ?? {}) as Record<string, unknown>;
    if (!isLngLat([marker.lng, marker.lat])) {
      return { ok: false, error: `markers[${index}] needs valid lng/lat.` };
    }
    markers.push({
      lng: marker.lng as number,
      lat: marker.lat as number,
      label: optionalString(marker.label),
      color: optionalString(marker.color),
    });
  }

  const lines: MapLine[] = [];
  for (const [index, item] of (Array.isArray(input.lines)
    ? input.lines
    : []
  ).entries()) {
    const line = (item ?? {}) as Record<string, unknown>;
    const coordinates = line.coordinates;
    if (
      !Array.isArray(coordinates) ||
      coordinates.length < 2 ||
      !coordinates.every(isLngLat)
    ) {
      return {
        ok: false,
        error: `lines[${index}] needs at least two [lng, lat] coordinates.`,
      };
    }
    lines.push({
      coordinates,
      label: optionalString(line.label),
      color: optionalString(line.color),
    });
  }

  if (input.center !== undefined && !isLngLat(input.center)) {
    return { ok: false, error: "center must be [lng, lat]." };
  }
  if (input.center === undefined && markers.length + lines.length === 0) {
    return { ok: false, error: "Map block needs a center, markers, or lines." };
  }

  const zoom = Number.isFinite(input.zoom)
    ? Math.min(22, Math.max(0, input.zoom as number))
    : undefined;
  const height = Number.isFinite(input.height)
    ? Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, input.height as number))
    : undefined;

  return {
    ok: true,
    spec: {
      center: input.center as LngLat | undefined,
      zoom,
      height,
      markers,
      lines,
    },
  };
}

/** Every coordinate in the spec, used to fit the initial viewport. */
export function mapSpecPoints(spec: MapSpec): LngLat[] {
  return [
    ...spec.markers.map((marker): LngLat => [marker.lng, marker.lat]),
    ...spec.lines.flatMap((line) => line.coordinates),
  ];
}
