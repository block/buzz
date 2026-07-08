import { openUrl } from "@tauri-apps/plugin-opener";
import type { ComponentType } from "react";
import { ExternalLink, RefreshCcw, RotateCw } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import { useUpdaterContext } from "./hooks/UpdaterProvider";
import type { UpdateStatus } from "./hooks/use-updater";

const indicatorButtonClass =
  "relative text-muted-foreground/80 hover:bg-muted/60 hover:text-foreground";

type IndicatorIcon = ComponentType<{
  "aria-hidden"?: boolean;
  className?: string;
}>;

const variants: Record<
  "available" | "downloading" | "installing" | "manual-required" | "ready",
  {
    Icon: IndicatorIcon;
    iconClassName?: string;
    label: string;
    badgeColor: string;
  }
> = {
  available: {
    Icon: RefreshCcw,
    label: "Update available",
    badgeColor: "bg-primary",
  },
  downloading: {
    Icon: Spinner,
    iconClassName: "h-4 w-4 border-2",
    label: "Downloading update\u2026",
    badgeColor: "bg-primary",
  },
  installing: {
    Icon: Spinner,
    iconClassName: "h-4 w-4 border-2",
    label: "Installing update\u2026",
    badgeColor: "bg-primary",
  },
  "manual-required": {
    Icon: ExternalLink,
    label:
      "Update available — download from GitHub (use AppImage for auto-updates)",
    badgeColor: "bg-primary",
  },
  ready: {
    Icon: RotateCw,
    label: "Restart to update",
    badgeColor: "bg-emerald-500",
  },
};

function getVariant(state: UpdateStatus["state"]) {
  if (
    state === "available" ||
    state === "downloading" ||
    state === "installing" ||
    state === "manual-required" ||
    state === "ready"
  ) {
    return variants[state];
  }
  return null;
}

export function UpdateIndicator({ className }: { className?: string }) {
  const { status, downloadAndInstall, relaunch } = useUpdaterContext();
  const variant = getVariant(status.state);

  if (!variant) {
    return null;
  }

  const { Icon, iconClassName = "h-4 w-4", label, badgeColor } = variant;
  const isActionable =
    status.state === "available" ||
    status.state === "ready" ||
    status.state === "manual-required";
  const handleClick =
    status.state === "ready"
      ? relaunch
      : status.state === "manual-required"
        ? () => {
            void openUrl(status.releaseUrl);
          }
        : status.state === "available"
          ? downloadAndInstall
          : null;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={label}
          className={`${indicatorButtonClass} ${className ?? ""}`}
          disabled={!isActionable}
          onClick={() => {
            if (handleClick) {
              void handleClick();
            }
          }}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Icon aria-hidden className={iconClassName} />
          <span
            className={`absolute right-1 top-1 h-1.5 w-1.5 rounded-full ${badgeColor} animate-pulse`}
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}
