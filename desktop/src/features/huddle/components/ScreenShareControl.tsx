import { invoke } from "@tauri-apps/api/core";
import {
  AlertCircle,
  AppWindow,
  Monitor,
  MonitorUp,
  Square,
} from "lucide-react";
import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/shared/ui/dialog";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { useHuddleScreen } from "../HuddleScreenContext";

type ScreenSource = {
  id: string;
  label: string;
  source_type: "display" | "window" | "portal";
};

/** Source selection always precedes native capture; no implicit whole-screen share. */
export function ScreenShareControl() {
  const { channelId, localShare, error: captureError } = useHuddleScreen();
  const [open, setOpen] = React.useState(false);
  const [sources, setSources] = React.useState<ScreenSource[]>([]);
  const [selected, setSelected] = React.useState("");
  const [loading, setLoading] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [errorOpen, setErrorOpen] = React.useState(false);
  const visibleError = error ?? captureError;
  const request = React.useRef(0);
  const sourceChannel = React.useRef<string | null>(null);

  React.useEffect(() => {
    setErrorOpen(Boolean(visibleError));
  }, [visibleError]);

  React.useEffect(() => {
    if (open && channelId !== sourceChannel.current) setOpen(false);
  }, [channelId, open]);

  React.useEffect(() => {
    if (!open) {
      request.current++;
      return;
    }
    const revision = ++request.current;
    setLoading(true);
    setError(null);
    setSelected("");
    setSources([]);
    void invoke<ScreenSource[]>("list_huddle_screen_sources")
      .then((entries) => {
        if (request.current === revision) setSources(entries);
      })
      .catch((failure) => {
        if (request.current === revision) setError(String(failure));
      })
      .finally(() => {
        if (request.current === revision) setLoading(false);
      });
    return () => {
      request.current++;
    };
  }, [open]);

  async function stop() {
    if (!localShare || busy) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("stop_huddle_screen_share", {
        sessionId: localShare.session_id,
      });
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(false);
    }
  }

  async function start() {
    if (!selected || busy) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("start_huddle_screen_share", {
        channelId: sourceChannel.current,
        sourceId: selected,
      });
      setOpen(false);
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            aria-label={localShare ? "Stop sharing screen" : "Share screen"}
            aria-pressed={Boolean(localShare)}
            aria-busy={busy}
            disabled={busy || !channelId}
            className="buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md"
            onClick={() => {
              if (localShare) {
                void stop();
              } else {
                sourceChannel.current = channelId;
                setOpen(true);
              }
            }}
            size="icon"
            type="button"
            variant={localShare ? "destructive" : "secondary"}
          >
            {localShare ? (
              <Square className="h-4 w-4" />
            ) : (
              <MonitorUp className="h-4 w-4" />
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent className="buzz-huddle-tooltip" side="top">
          {localShare ? `Stop sharing ${localShare.label}` : "Share screen"}
        </TooltipContent>
      </Tooltip>
      {!open && visibleError ? (
        <Popover open={errorOpen} onOpenChange={setErrorOpen}>
          <Tooltip>
            <TooltipTrigger asChild>
              <PopoverTrigger asChild>
                <Button
                  aria-label="Screen sharing error"
                  size="icon"
                  variant="ghost"
                  className="h-8 w-8 shrink-0 text-destructive"
                >
                  <AlertCircle className="h-4 w-4" />
                </Button>
              </PopoverTrigger>
            </TooltipTrigger>
            <TooltipContent>Screen sharing error</TooltipContent>
          </Tooltip>
          <PopoverContent side="top" className="max-w-xs rounded-md">
            <p role="alert" className="break-words text-sm">
              {visibleError}
            </p>
          </PopoverContent>
        </Popover>
      ) : null}
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!busy) setOpen(value);
        }}
      >
        <DialogContent
          className="max-w-xl rounded-lg"
          aria-describedby={undefined}
        >
          <DialogTitle className="tracking-normal">Share Screen</DialogTitle>
          {loading ? (
            <output aria-live="polite" className="text-sm">
              Loading sources...
            </output>
          ) : null}
          {!loading && sources.length === 0 && !error ? (
            <p className="text-sm text-muted-foreground">
              No screens or windows are available.
            </p>
          ) : null}
          <fieldset className="grid max-h-80 gap-1 overflow-y-auto">
            <legend className="sr-only">Screen or window</legend>
            {sources.map((source) => (
              <label
                key={source.id}
                className="flex min-w-0 cursor-pointer items-center gap-3 rounded-md px-3 py-2 hover:bg-accent"
              >
                <input
                  type="radio"
                  name="screen-source"
                  value={source.id}
                  checked={selected === source.id}
                  onChange={() => setSelected(source.id)}
                  disabled={busy}
                />
                {source.source_type !== "window" ? (
                  <Monitor aria-hidden className="h-5 w-5 shrink-0" />
                ) : (
                  <AppWindow aria-hidden className="h-5 w-5 shrink-0" />
                )}
                <span className="min-w-0 break-words text-sm">
                  {source.label}
                </span>
              </label>
            ))}
          </fieldset>
          {error ? (
            <p role="alert" className="break-words text-sm text-destructive">
              {error}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button
              disabled={busy}
              onClick={() => setOpen(false)}
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              disabled={!selected || busy}
              aria-busy={busy}
              onClick={() => void start()}
            >
              <MonitorUp className="mr-2 h-4 w-4" />
              Share
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
