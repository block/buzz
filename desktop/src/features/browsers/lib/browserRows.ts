import type { BrowserAgentGrant } from "@/features/browser-agent/lib/types";
import type {
  PlaygroundBrowser,
  PlaygroundSession,
} from "@/features/playground/lib/sessions";
import { mainTabSid } from "@/features/playground/lib/browserGroups";

export type BrowserHost = "main" | "windowed";

/** Secondary tab under a browser group (URL + thumbnail only in the list). */
export type BrowserListSecondaryTab = {
  surfaceId: string;
  title: string;
  url: string;
};

export type BrowserListRow = {
  key: string;
  /** Browser group id (one left-nav row per group). */
  browserId: string;
  /**
   * Active tab surface id (Drive / grant target). May differ from main when a
   * secondary tab is focused.
   */
  surfaceId: string;
  /** Main tab (tabSids[0]) — title, screenshot, Open target for the main row. */
  mainSurfaceId: string;
  title: string;
  url: string;
  host: BrowserHost;
  /** Webview window label segment ("main" or OS/embed popout label). */
  windowLabel: string;
  /** OS/embed popout label when host is windowed. */
  popoutLabel?: string;
  tabCount: number;
  /** Extra tabs after main — indented subrows (URL + thumb only). */
  secondaryTabs: BrowserListSecondaryTab[];
};

export type DetachedBrowserHost = {
  label: string;
  title: string;
  playgroundSid: string;
};

export function browserRowUrlLabel(url: string, max = 64): string {
  const trimmed = url.trim();
  if (trimmed.length <= max) return trimmed;
  return `${trimmed.slice(0, max - 1)}…`;
}

/** Match grant by surfaceId. Canonical label is `playground-{sid}` on every host. */
export function findGrantForRow(
  grants: readonly BrowserAgentGrant[],
  row: Pick<BrowserListRow, "surfaceId" | "windowLabel">,
): BrowserAgentGrant | null {
  const mainLabel = `playground-${row.surfaceId}`;
  const windowedLabel = `playground-${row.surfaceId}--${row.windowLabel}`;
  const exact = grants.find((grant) => {
    if (grant.surface !== "playground" || grant.surfaceId !== row.surfaceId) {
      return false;
    }
    // Single live webview: grants sit on the canonical label.
    if (grant.webviewLabel === mainLabel) return true;
    if (row.windowLabel === "main") return false;
    return (
      grant.webviewLabel === windowedLabel ||
      grant.webviewLabel.endsWith(`--${row.windowLabel}`)
    );
  });
  if (exact) return exact;
  return (
    grants.find(
      (grant) =>
        grant.surface === "playground" && grant.surfaceId === row.surfaceId,
    ) ?? null
  );
}

export function buildBrowserListRows(input: {
  sessions: readonly PlaygroundSession[];
  browsers?: readonly PlaygroundBrowser[];
  detached: readonly DetachedBrowserHost[];
}): BrowserListRow[] {
  const sessionBySid = new Map(
    input.sessions.map((session) => [session.sid, session]),
  );
  const playgroundHostBySid = new Map<string, DetachedBrowserHost>();
  for (const row of input.detached) {
    playgroundHostBySid.set(row.playgroundSid, row);
  }

  const browsers =
    input.browsers && input.browsers.length > 0
      ? input.browsers
      : input.sessions.map((session) => ({
          browserId: session.sid,
          tabSids: [session.sid],
          activeTabSid: session.sid,
        }));

  const rows: BrowserListRow[] = [];
  for (const browser of browsers) {
    const mainSid = mainTabSid(browser);
    const main =
      sessionBySid.get(mainSid) ??
      sessionBySid.get(browser.tabSids[0] ?? "") ??
      null;
    if (!main) continue;
    const active =
      sessionBySid.get(browser.activeTabSid) ?? main;
    // Prefer detached host of the active tab (same group cannot split hosts in MVP).
    const detached =
      playgroundHostBySid.get(active.sid) ??
      playgroundHostBySid.get(main.sid);
    const secondaryTabs: BrowserListSecondaryTab[] = [];
    for (const sid of browser.tabSids.slice(1)) {
      const session = sessionBySid.get(sid);
      if (!session) continue;
      secondaryTabs.push({
        surfaceId: session.sid,
        title: session.name,
        url: session.url,
      });
    }
    rows.push({
      key: detached
        ? `browser:${browser.browserId}:window:${detached.label}`
        : `browser:${browser.browserId}`,
      browserId: browser.browserId,
      surfaceId: active.sid,
      mainSurfaceId: main.sid,
      title: main.name,
      url: main.url,
      host: detached ? "windowed" : "main",
      windowLabel: detached ? detached.label : "main",
      tabCount: browser.tabSids.length,
      secondaryTabs,
      ...(detached ? { popoutLabel: detached.label } : {}),
    });
  }
  return rows;
}

/** Compact Browsers-row chip when the WKWebView is not running yet. */
export const BROWSER_ROW_COLD_CHIP = "Cold";

/**
 * Liveness chip for a Browsers list row.
 * Pass `webviewOpen` from `isPlaygroundWebviewOpen` (native get_webview),
 * not screenshot/favicon success — screenshot fails on non-macOS stubs too.
 */
export function browserRowLivenessChip(webviewOpen: boolean): string | null {
  return webviewOpen ? null : BROWSER_ROW_COLD_CHIP;
}

/** Main rows show Open (slide-out); detached rows never show it. */
export function browserRowShowsOpenButton(
  row: Pick<BrowserListRow, "host">,
): boolean {
  return row.host === "main";
}

