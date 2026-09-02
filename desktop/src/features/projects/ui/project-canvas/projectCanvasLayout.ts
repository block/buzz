import {
  PROJECT_CANVAS_LAYOUT_COORDINATE_LIMIT,
  PROJECT_CANVAS_LAYOUT_MIN_WIDGET_SIZE,
  PROJECT_CANVAS_MAX_LAYOUT_WIDGETS,
  type ProjectCanvasDashboardLayout,
  type ProjectCanvasLayoutPoint,
  type ProjectCanvasLayouts,
  type ProjectCanvasLayoutSize,
} from "./projectCanvasProtocol";

/**
 * Per-community, per-project widget arrangement, mirroring the storage shape
 * of {@link ./projectCanvasConsent}. Layout is deliberately *not* keyed by
 * package revision: an agent editing the package must not throw away where the
 * user put things. Only overrides are stored, so an untouched widget keeps
 * following the package default even after a later revision moves it.
 *
 * Storage failures degrade to package defaults on the next load — this is
 * refreshable preference state, not a durable record.
 */

const STORAGE_VERSION = 1;
const MAX_DASHBOARD_ID_LENGTH = 128;
const WIDGET_ID_PATTERN = /^[A-Za-z0-9._-]{1,128}$/;

export const PROJECT_CANVAS_MAX_STORED_LAYOUT_DASHBOARDS = 32;
export const PROJECT_CANVAS_MAX_LAYOUT_RECORD_BYTES = 128 * 1_024;

type StoredDashboardLayout = ProjectCanvasDashboardLayout & {
  dashboard: string;
};

export function projectCanvasLayoutStorageKey(
  communityId: string,
  projectId: string,
): string {
  return `buzz.projectCanvasLayout.${communityId} ${projectId}`;
}

function byteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function sanitizeCoordinate(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  if (Math.abs(value) > PROJECT_CANVAS_LAYOUT_COORDINATE_LIMIT) return null;
  return value;
}

function sanitizePoint(value: unknown): ProjectCanvasLayoutPoint | null {
  if (!value || typeof value !== "object") return null;
  const x = sanitizeCoordinate((value as { x?: unknown }).x);
  const y = sanitizeCoordinate((value as { y?: unknown }).y);
  return x === null || y === null ? null : { x, y };
}

function sanitizeDimension(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  if (
    value < PROJECT_CANVAS_LAYOUT_MIN_WIDGET_SIZE ||
    value > PROJECT_CANVAS_LAYOUT_COORDINATE_LIMIT
  ) {
    return null;
  }
  return value;
}

function sanitizeSize(value: unknown): ProjectCanvasLayoutSize | null {
  if (!value || typeof value !== "object") return null;
  const width = sanitizeDimension((value as { width?: unknown }).width);
  const height = sanitizeDimension((value as { height?: unknown }).height);
  return width === null || height === null ? null : { height, width };
}

function sanitizeOverrides<T>(
  value: unknown,
  sanitizeEntry: (entry: unknown) => T | null,
): Record<string, T> {
  if (!value || typeof value !== "object") return {};
  const entries: Array<[string, T]> = [];
  for (const [widgetId, entry] of Object.entries(value)) {
    if (entries.length >= PROJECT_CANVAS_MAX_LAYOUT_WIDGETS) break;
    if (!WIDGET_ID_PATTERN.test(widgetId)) continue;
    const sanitized = sanitizeEntry(entry);
    if (sanitized) entries.push([widgetId, sanitized]);
  }
  // Object.fromEntries defines own properties, so a widget named `__proto__`
  // cannot reach the prototype chain.
  return Object.fromEntries(entries);
}

function sanitizeWidgets(
  value: unknown,
): Record<string, ProjectCanvasLayoutPoint> {
  return sanitizeOverrides(value, sanitizePoint);
}

function sanitizeSizes(
  value: unknown,
): Record<string, ProjectCanvasLayoutSize> {
  return sanitizeOverrides(value, sanitizeSize);
}

function isEmptyLayout(layout: ProjectCanvasDashboardLayout): boolean {
  return (
    !layout.pan &&
    Object.keys(layout.widgets).length === 0 &&
    Object.keys(layout.sizes).length === 0
  );
}

/** Oldest written first, so pruning drops the least recently written. */
function readStoredDashboards(storageKey: string): StoredDashboardLayout[] {
  let raw: string | null = null;
  try {
    raw = window.localStorage.getItem(storageKey);
  } catch {
    return [];
  }
  if (!raw || byteLength(raw) > PROJECT_CANVAS_MAX_LAYOUT_RECORD_BYTES) {
    return [];
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  const dashboards = (parsed as { dashboards?: unknown } | null)?.dashboards;
  if (!Array.isArray(dashboards)) return [];

  const stored: StoredDashboardLayout[] = [];
  const seen = new Set<string>();
  for (const entry of dashboards.slice(
    -PROJECT_CANVAS_MAX_STORED_LAYOUT_DASHBOARDS,
  )) {
    const dashboard = (entry as { dashboard?: unknown } | null)?.dashboard;
    if (
      typeof dashboard !== "string" ||
      dashboard.length === 0 ||
      dashboard.length > MAX_DASHBOARD_ID_LENGTH ||
      seen.has(dashboard)
    ) {
      continue;
    }
    const layout: ProjectCanvasDashboardLayout = {
      pan: sanitizePoint((entry as { pan?: unknown }).pan),
      sizes: sanitizeSizes((entry as { sizes?: unknown }).sizes),
      widgets: sanitizeWidgets((entry as { widgets?: unknown }).widgets),
    };
    if (isEmptyLayout(layout)) continue;
    seen.add(dashboard);
    stored.push({ dashboard, ...layout });
  }
  return stored;
}

export function readProjectCanvasLayouts(
  communityId: string,
  projectId: string,
): ProjectCanvasLayouts {
  return Object.fromEntries(
    readStoredDashboards(
      projectCanvasLayoutStorageKey(communityId, projectId),
    ).map(({ dashboard, pan, sizes, widgets }) => [
      dashboard,
      { pan, sizes, widgets },
    ]),
  );
}

/**
 * Replaces one dashboard's entry wholesale, so widget ids the package no
 * longer declares disappear on the next save instead of accumulating.
 */
export function writeProjectCanvasDashboardLayout(
  communityId: string,
  projectId: string,
  dashboard: string,
  layout: ProjectCanvasDashboardLayout,
): void {
  if (dashboard.length === 0 || dashboard.length > MAX_DASHBOARD_ID_LENGTH) {
    return;
  }
  const storageKey = projectCanvasLayoutStorageKey(communityId, projectId);
  const dashboards = readStoredDashboards(storageKey).filter(
    (entry) => entry.dashboard !== dashboard,
  );
  const next: ProjectCanvasDashboardLayout = {
    pan: sanitizePoint(layout.pan),
    sizes: sanitizeSizes(layout.sizes),
    widgets: sanitizeWidgets(layout.widgets),
  };
  if (!isEmptyLayout(next)) dashboards.push({ dashboard, ...next });
  while (dashboards.length > PROJECT_CANVAS_MAX_STORED_LAYOUT_DASHBOARDS) {
    dashboards.shift();
  }

  try {
    if (dashboards.length === 0) {
      window.localStorage.removeItem(storageKey);
      return;
    }
    let serialized = JSON.stringify({
      dashboards,
      version: STORAGE_VERSION,
    });
    while (
      byteLength(serialized) > PROJECT_CANVAS_MAX_LAYOUT_RECORD_BYTES &&
      dashboards.length > 1
    ) {
      dashboards.shift();
      serialized = JSON.stringify({ dashboards, version: STORAGE_VERSION });
    }
    // A single dashboard over the cap keeps the last good record rather than
    // replacing it with something unreadable.
    if (byteLength(serialized) > PROJECT_CANVAS_MAX_LAYOUT_RECORD_BYTES) return;
    window.localStorage.setItem(storageKey, serialized);
  } catch {
    // Storage failures degrade to package defaults on the next load.
  }
}
