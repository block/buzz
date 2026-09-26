import { Download, Share2 } from "lucide-react";
import type * as React from "react";

import type { SiteRunbookRef } from "@/features/site-runbook/lib/types";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

import { exportBrowserShareFromRef } from "../lib/exportActions";
import type { BrowserShareSource } from "../lib/types";
import { BROWSER_SHARE_SECURITY_NOTE } from "../lib/types";

export function ExportBrowserShareButton({
  url,
  title,
  source,
  runbookRef,
  testId,
  label = "Export",
  variant = "ghost",
  size = "xs",
}: {
  url: string;
  title?: string;
  source: BrowserShareSource;
  runbookRef: SiteRunbookRef;
  testId?: string;
  label?: string;
  variant?: React.ComponentProps<typeof Button>["variant"];
  size?: React.ComponentProps<typeof Button>["size"];
}) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label={`Export ${title ?? "browser"} share`}
          data-testid={testId ?? "browser-share-export"}
          size={size}
          title={BROWSER_SHARE_SECURITY_NOTE}
          type="button"
          variant={variant}
        >
          <Share2 className="mr-1 h-3 w-3" />
          {label}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-w-xs">
        <DropdownMenuItem
          data-testid={
            testId ? `${testId}-download` : "browser-share-export-download"
          }
          onClick={() =>
            exportBrowserShareFromRef({
              url,
              title,
              source,
              runbookRef,
              mode: "download",
            })
          }
        >
          <Download className="mr-2 h-4 w-4" />
          Download JSON
        </DropdownMenuItem>
        <DropdownMenuItem
          data-testid={testId ? `${testId}-copy` : "browser-share-export-copy"}
          onClick={() =>
            exportBrowserShareFromRef({
              url,
              title,
              source,
              runbookRef,
              mode: "clipboard",
            })
          }
        >
          <Share2 className="mr-2 h-4 w-4" />
          Copy JSON
        </DropdownMenuItem>
        <p className="px-2 py-1.5 text-2xs text-muted-foreground">
          {BROWSER_SHARE_SECURITY_NOTE}
        </p>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
