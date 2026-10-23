import {
  AlertCircle,
  ChevronDown,
  RefreshCw,
  Video,
  VideoOff,
} from "lucide-react";
import * as React from "react";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { useHuddleCamera } from "../HuddleCameraContext";

/** Camera availability never implies consent; enabling remains an explicit action. */
export function CameraControl() {
  const {
    channelId,
    localShare,
    selectedDeviceId,
    cameras,
    checking,
    busy,
    error,
    request,
    refresh,
  } = useHuddleCamera();
  const [selected, setSelected] = React.useState("");
  const [errorOpen, setErrorOpen] = React.useState(false);
  React.useEffect(() => setErrorOpen(Boolean(error)), [error]);
  const enabled = !!localShare;
  const available = cameras.length > 0;
  const label = enabled
    ? "Turn camera off"
    : checking
      ? "Checking cameras"
      : available
        ? "Turn camera on"
        : "No camera detected";
  const cameraLabel =
    cameras.find((camera) => camera.deviceId === selected)?.label ?? "Camera";
  return (
    <>
      <div className="flex shrink-0 items-center gap-px">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label={label}
              aria-pressed={enabled}
              aria-busy={busy}
              disabled={
                !channelId || (!enabled && (checking || !available || busy))
              }
              className="buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md"
              size="icon"
              type="button"
              variant={enabled ? "secondary" : "ghost"}
              onClick={() =>
                void request(enabled ? "stop" : "start", selected, cameraLabel)
              }
            >
              {enabled ? (
                <Video className="h-4 w-4" />
              ) : (
                <VideoOff className="h-4 w-4" />
              )}
            </Button>
          </TooltipTrigger>
          <TooltipContent className="buzz-huddle-tooltip">
            {label}
          </TooltipContent>
        </Tooltip>
        <DropdownMenu
          onOpenChange={(open) => {
            if (open) void refresh();
          }}
        >
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <Button
                  aria-label="Camera settings"
                  size="icon"
                  type="button"
                  variant="ghost"
                  className="h-12 w-6 shrink-0 rounded-md"
                  disabled={!channelId}
                >
                  <ChevronDown className="h-3 w-3" />
                </Button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent className="buzz-huddle-tooltip">
              Camera settings
            </TooltipContent>
          </Tooltip>
          <DropdownMenuContent side="top" className="max-w-xs">
            <DropdownMenuLabel>Cameras</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={enabled ? selectedDeviceId : selected}
              onValueChange={(deviceId) => {
                setSelected(deviceId);
                if (enabled)
                  void request(
                    "start",
                    deviceId,
                    cameras.find((camera) => camera.deviceId === deviceId)
                      ?.label ?? "Camera",
                  );
              }}
            >
              <DropdownMenuRadioItem value="" disabled={busy}>
                System default
              </DropdownMenuRadioItem>
              {cameras
                .filter((camera) => camera.deviceId)
                .map((camera) => (
                  <DropdownMenuRadioItem
                    key={camera.deviceId}
                    value={camera.deviceId}
                    disabled={busy}
                  >
                    <span className="break-words">{camera.label}</span>
                  </DropdownMenuRadioItem>
                ))}
            </DropdownMenuRadioGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={refresh}>
              <RefreshCw className="mr-2 h-4 w-4" />
              Refresh cameras
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {error ? (
        <Popover open={errorOpen} onOpenChange={setErrorOpen}>
          <PopoverTrigger asChild>
            <Button
              aria-label="Camera error"
              size="icon"
              variant="ghost"
              className="h-8 w-8 shrink-0 text-destructive"
            >
              <AlertCircle className="h-4 w-4" />
            </Button>
          </PopoverTrigger>
          <PopoverContent side="top" className="max-w-xs rounded-md">
            <p role="alert" className="break-words text-sm">
              {error}
            </p>
          </PopoverContent>
        </Popover>
      ) : null}
    </>
  );
}
