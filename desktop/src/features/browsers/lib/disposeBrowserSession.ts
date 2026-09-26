import { clearBrowserAgentGrant } from "@/features/browser-agent/lib/api";
import type { BrowserAgentGrant } from "@/features/browser-agent/lib/types";
import {
  disposePlayground,
  disposePlaygroundBrowser,
  getBrowserForSid,
  getPlaygroundBrowser,
} from "@/features/playground/lib/sessions";
import { closeEmbeddedWindow } from "@/features/popout/lib/embeddedWindows";
import { closePopoutWindow } from "@/features/popout/lib/popoutWindows";

import { sidRunbookRef } from "@/features/site-runbook/lib/keys";
import { clearSiteRunbook } from "@/features/site-runbook/lib/store";

import type { BrowserListRow } from "./browserRows";
import { findGrantForRow } from "./browserRows";

export type DisposeBrowserSessionHost = "embed" | "os" | "main";

/**
 * Tear down a Browsers list row: clear Observe/Drive grants for the surface,
 * close embed or OS pop-out when that was the host, dispose the playground
 * session (hides overlay, destroys webview on main, removes from list).
 */
export async function disposeBrowserSession(input: {
  row: BrowserListRow;
  grants: readonly BrowserAgentGrant[];
  isEmbeddedLabel: (label: string) => boolean;
}): Promise<DisposeBrowserSessionHost> {
  const { row, grants, isEmbeddedLabel } = input;

  const browserId =
    typeof row.browserId === "string" && row.browserId
      ? row.browserId
      : getBrowserForSid(row.surfaceId)?.browserId;
  const group = browserId
    ? getPlaygroundBrowser(browserId) ?? getBrowserForSid(row.surfaceId)
    : getBrowserForSid(row.surfaceId);
  const surfaceIds = new Set<string>(
    group?.tabSids?.length ? group.tabSids : [row.surfaceId],
  );
  surfaceIds.add(row.surfaceId);
  const surfaceGrants = grants.filter(
    (grant) =>
      grant.surface === "playground" && surfaceIds.has(grant.surfaceId),
  );
  const primary = findGrantForRow(grants, row);
  const labels = new Set<string>();
  if (primary) labels.add(primary.webviewLabel);
  for (const grant of surfaceGrants) {
    labels.add(grant.webviewLabel);
  }

  for (const webviewLabel of labels) {
    try {
      await clearBrowserAgentGrant(webviewLabel);
    } catch {
      // Best-effort: webview/session teardown still proceeds.
    }
  }

  let host: DisposeBrowserSessionHost = "main";
  if (row.host === "windowed") {
    const label = row.popoutLabel ?? row.windowLabel;
    if (label && label !== "main") {
      if (isEmbeddedLabel(label)) {
        closeEmbeddedWindow(label);
        host = "embed";
      } else {
        await closePopoutWindow(label);
        host = "os";
      }
    }
  }

  // Remove disposes the whole browser group (all tabs).
  // Pin cleanup by session id runs inside disposePlaygroundSessionOnly
  // (unpinPlaygroundSessionEverywhere) for each tab sid.
  if (browserId) {
    disposePlaygroundBrowser(browserId);
  } else {
    disposePlayground(row.surfaceId);
  }
  for (const sid of surfaceIds) {
    clearSiteRunbook(sidRunbookRef(sid));
  }
  return host;
}
