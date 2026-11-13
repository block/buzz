import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, Sparkles, X } from "lucide-react";

import {
  DEPRECATION_BANNER_STORAGE_KEY,
  shouldShowDeprecationBanner,
} from "@/app/deprecationBannerVisibility";
import { Button } from "@/shared/ui/button";

const BUZZ_APP_RELEASES_URL =
  "https://github.com/block/buzz-app/releases/latest";
const BUZZ_APP_UPGRADE_GUIDE_URL =
  "https://github.com/block/buzz-app/blob/main/docs/upgrading.md";

function readDismissedAt(): string | null {
  try {
    return window.localStorage.getItem(DEPRECATION_BANNER_STORAGE_KEY);
  } catch {
    return null;
  }
}

export function DeprecationBanner() {
  const [isVisible, setIsVisible] = React.useState(() =>
    shouldShowDeprecationBanner(readDismissedAt(), Date.now()),
  );

  if (!isVisible) {
    return null;
  }

  const handleDismiss = () => {
    try {
      window.localStorage.setItem(
        DEPRECATION_BANNER_STORAGE_KEY,
        String(Date.now()),
      );
    } catch {
      // Storage unavailable: hide for this session only.
    }
    setIsVisible(false);
  };

  return (
    <div
      className="flex shrink-0 items-center gap-3 border-b border-primary/20 bg-primary/10 px-4 py-2 text-sm text-foreground"
      data-testid="deprecation-banner"
      role="status"
    >
      <Sparkles aria-hidden className="size-4 shrink-0 text-primary" />
      {/* The actions wrap under the message when the row gets too narrow. */}
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1.5">
        <p
          className="min-w-0 grow basis-64"
          data-testid="deprecation-banner-message"
        >
          A new Buzz app is here! This version is being retired, so grab the new
          one whenever you get a chance.
        </p>
        <div className="flex shrink-0 items-center gap-2">
          {/* The outline variant's own border is too faint to see on the
              banner tint, so this one uses a stronger border color. */}
          <Button
            aria-label="More info about upgrading, opens in your browser"
            className="h-7 shrink-0 border-foreground/70"
            data-testid="deprecation-banner-upgrade-guide"
            onClick={() => {
              void openUrl(BUZZ_APP_UPGRADE_GUIDE_URL);
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            More info
          </Button>
          <Button
            className="h-7 shrink-0 gap-1.5"
            data-testid="deprecation-banner-download"
            onClick={() => {
              void openUrl(BUZZ_APP_RELEASES_URL);
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            Get the new app
            <ExternalLink aria-hidden className="size-3.5" />
          </Button>
        </div>
      </div>
      <Button
        aria-label="Remind me later"
        className="size-7 shrink-0 text-muted-foreground"
        data-testid="deprecation-banner-dismiss"
        onClick={handleDismiss}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X className="size-4" />
      </Button>
    </div>
  );
}
