import { toast } from "sonner";

import type { SiteRunbook } from "@/features/site-runbook/lib/types";
import { getSiteRunbookOrEmpty } from "@/features/site-runbook/lib/store";
import type { SiteRunbookRef } from "@/features/site-runbook/lib/types";

import { copyBrowserShareToClipboard, downloadBrowserShare } from "./download";
import { buildBrowserShare } from "./serialize";
import type { BrowserShareSource } from "./types";

export function exportBrowserShareFromRef(input: {
  url: string;
  title?: string;
  source: BrowserShareSource;
  runbookRef: SiteRunbookRef;
  mode: "download" | "clipboard";
}): boolean {
  const runbook: SiteRunbook = getSiteRunbookOrEmpty(input.runbookRef);
  const share = buildBrowserShare({
    url: input.url,
    title: input.title,
    source: input.source,
    runbook,
  });
  if (!share) {
    toast.error("Could not export — check the URL.");
    return false;
  }
  if (input.mode === "download") {
    downloadBrowserShare(share);
    toast.success("Browser share downloaded");
    return true;
  }
  void copyBrowserShareToClipboard(share)
    .then(() => toast.success("Browser share copied"))
    .catch(() => toast.error("Could not copy share"));
  return true;
}
