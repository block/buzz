import * as React from "react";

import type { RemoteHostConnection } from "../types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

type RemoteHostSettingsDialogProps = {
  open: boolean;
  initial: RemoteHostConnection | null;
  onOpenChange: (open: boolean) => void;
  onSave: (conn: RemoteHostConnection) => void;
  onClear: () => void;
};

export function RemoteHostSettingsDialog({
  open,
  initial,
  onOpenChange,
  onSave,
  onClear,
}: RemoteHostSettingsDialogProps) {
  const [label, setLabel] = React.useState(initial?.label ?? "home");
  const [baseUrl, setBaseUrl] = React.useState(
    initial?.baseUrl ?? "http://100.79.175.63:8787",
  );
  const [token, setToken] = React.useState(initial?.token ?? "");
  const [defaultRoom, setDefaultRoom] = React.useState(
    initial?.defaultRoom ?? "92297894-c2e8-4df1-a710-d1cfd1032d5e",
  );

  React.useEffect(() => {
    if (!open) return;
    setLabel(initial?.label ?? "home");
    setBaseUrl(initial?.baseUrl ?? "http://100.79.175.63:8787");
    setToken(initial?.token ?? "");
    setDefaultRoom(
      initial?.defaultRoom ?? "92297894-c2e8-4df1-a710-d1cfd1032d5e",
    );
  }, [open, initial]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="max-w-md"
        data-testid="remote-host-settings-dialog"
      >
        <DialogHeader>
          <DialogTitle>Remote host connection</DialogTitle>
          <DialogDescription>
            Connect to headless <code className="text-xs">host-agentd</code> on
            home over Tailscale (mesh IP, e.g.{" "}
            <code className="text-xs">http://100.79.175.63:8787</code>). Token
            is stored locally in this profile — do not paste it into Buzz rooms.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3 py-2">
          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-host-label"
            >
              Label
            </label>
            <Input
              id="remote-host-label"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-host-url"
            >
              Base URL
            </label>
            <Input
              id="remote-host-url"
              placeholder="http://100.79.175.63:8787"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-host-token"
            >
              Bearer token
            </label>
            <Input
              id="remote-host-token"
              type="password"
              autoComplete="off"
              placeholder="HOST_AGENTD_TOKEN"
              value={token}
              onChange={(e) => setToken(e.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <label
              className="text-xs font-medium text-foreground"
              htmlFor="remote-host-room"
            >
              Default room UUID (for Arm)
            </label>
            <Input
              id="remote-host-room"
              placeholder="92297894-c2e8-4df1-a710-d1cfd1032d5e"
              value={defaultRoom}
              onChange={(e) => setDefaultRoom(e.target.value)}
            />
          </div>
        </div>
        <DialogFooter className="gap-2 sm:gap-0">
          <Button
            type="button"
            variant="ghost"
            onClick={() => {
              onClear();
              onOpenChange(false);
            }}
          >
            Clear
          </Button>
          <Button
            type="button"
            onClick={() => {
              onSave({ label, baseUrl, token, defaultRoom });
              onOpenChange(false);
            }}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
