import * as React from "react";
import { getVersion } from "@tauri-apps/api/app";

import { RECENT_STARTUP_CHANGELOG } from "@/app/startupChangelog";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Button } from "@/shared/ui/button";

// The dialog is mounted alongside the community view. Community recovery and
// onboarding transitions can remount that subtree while the app process stays
// alive, so keep dismissal outside React state to avoid reopening the modal.
let dismissedForProcess = false;

export function StartupChangelogDialog() {
  const isE2e = Boolean(
    (window as Window & { __BUZZ_E2E__?: unknown }).__BUZZ_E2E__,
  );
  const [open, setOpen] = React.useState(() => !dismissedForProcess);
  const [version, setVersion] = React.useState<string | null>(null);
  const relayConnectionState = useRelayConnection();
  const relayIsHealthy =
    relayConnectionState === "connected" || relayConnectionState === "idle";

  const dismiss = React.useCallback(() => {
    dismissedForProcess = true;
    setOpen(false);
  }, []);

  const handleOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      if (!nextOpen) {
        dismiss();
        return;
      }
      setOpen(true);
    },
    [dismiss],
  );

  React.useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  if (isE2e) {
    return null;
  }

  return (
    <Dialog modal={relayIsHealthy} onOpenChange={handleOpenChange} open={open}>
      <DialogContent
        className="max-h-[85vh] overflow-y-auto sm:max-w-lg"
        overlayClassName={
          relayIsHealthy
            ? undefined
            : "pointer-events-none bg-transparent backdrop-blur-none"
        }
      >
        <DialogHeader>
          <DialogTitle>更新日志{version ? ` · v${version}` : ""}</DialogTitle>
          <DialogDescription>最近 10 日的 Buzz 修改记录</DialogDescription>
        </DialogHeader>
        <div className="grid gap-5 py-2">
          {RECENT_STARTUP_CHANGELOG.map((section) => (
            <section className="grid gap-2" key={section.date}>
              <h3 className="text-sm font-semibold">{section.date}</h3>
              <ul className="grid list-disc gap-1.5 pl-5 text-sm text-muted-foreground">
                {section.items.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            </section>
          ))}
        </div>
        <DialogFooter>
          <DialogClose asChild>
            <Button onClick={dismiss} type="button">
              知道了
            </Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
