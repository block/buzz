import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Sparkles, X } from "lucide-react";

import {
  DEPRECATION_BANNER_STORAGE_KEY,
  shouldShowDeprecationBanner,
} from "@/app/deprecationBannerVisibility";
import { NewAppDownloadButton } from "@/app/NewAppDownloadButton";
import {
  BUZZ_APP_DOWNLOADS_MANIFEST_URL,
  NEW_APP_MANIFEST_TIMEOUT_MS,
  resolveInstallerUrl,
} from "@/app/newAppInstaller";
import { getNewAppInstallerTarget } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";

const BUZZ_APP_UPGRADE_GUIDE_URL =
  "https://github.com/block/buzz-app/blob/main/docs/upgrading.md";

// Both banner actions share one look: text only, no icon.
const BANNER_ACTION_CLASS = "h-7 shrink-0";

function readDismissedAt(): string | null {
  try {
    return window.localStorage.getItem(DEPRECATION_BANNER_STORAGE_KEY);
  } catch {
    return null;
  }
}

/**
 * Looks up the new app's installer for this machine. Rejects on any failure;
 * resolves null when no installer is published for this platform.
 */
async function fetchInstallerUrl(signal: AbortSignal): Promise<string | null> {
  const target = await getNewAppInstallerTarget();
  if (!target) {
    return null;
  }
  const response = await fetch(BUZZ_APP_DOWNLOADS_MANIFEST_URL, { signal });
  if (!response.ok) {
    throw new Error(`installer manifest responded ${response.status}`);
  }
  const url = resolveInstallerUrl(await response.json(), target);
  if (!url) {
    throw new Error(`installer manifest has no usable entry for ${target}`);
  }
  return url;
}

export function DeprecationBanner() {
  const [isVisible, setIsVisible] = React.useState(() =>
    shouldShowDeprecationBanner(readDismissedAt(), Date.now()),
  );
  const [installerUrl, setInstallerUrl] = React.useState<string | null>(null);

  // One attempt per mount. The click never waits on it: until it resolves,
  // and whenever it fails, the button opens the website instead.
  React.useEffect(() => {
    if (!isVisible) {
      return;
    }
    let isCurrent = true;
    const controller = new AbortController();
    const timeout = window.setTimeout(
      () => controller.abort(new Error("installer manifest timed out")),
      NEW_APP_MANIFEST_TIMEOUT_MS,
    );
    fetchInstallerUrl(controller.signal)
      .then((url) => {
        if (isCurrent) {
          setInstallerUrl(url);
        }
      })
      .catch((error: unknown) => {
        if (isCurrent) {
          console.warn("Could not resolve the new app installer:", error);
        }
      })
      .finally(() => window.clearTimeout(timeout));
    return () => {
      isCurrent = false;
      window.clearTimeout(timeout);
      controller.abort();
    };
  }, [isVisible]);

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
          <Button
            aria-label="More info about upgrading, opens in your browser"
            className={BANNER_ACTION_CLASS}
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
          <NewAppDownloadButton
            className={BANNER_ACTION_CLASS}
            installerUrl={installerUrl}
          />
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
