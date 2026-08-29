import * as React from "react";
import { getVersion } from "@tauri-apps/api/app";
import { X } from "lucide-react";

import { RECENT_STARTUP_CHANGELOG } from "@/app/startupChangelog";
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

  const dismiss = React.useCallback(() => {
    dismissedForProcess = true;
    setOpen(false);
  }, []);

  React.useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  React.useEffect(() => {
    if (!open) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") dismiss();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [dismiss, open]);

  if (isE2e) {
    return null;
  }

  if (!open) return null;

  return (
    <div
      aria-label="更新日志"
      className="pointer-events-none fixed inset-0 z-40 flex items-center justify-center p-4"
      role="dialog"
    >
      <section className="pointer-events-auto relative grid max-h-[85vh] w-[calc(100vw-2rem)] max-w-lg grid-rows-[auto_minmax(0,1fr)_auto] overflow-hidden rounded-2xl bg-background p-6 shadow-2xl">
        <button
          aria-label="关闭更新日志"
          className="absolute right-4 top-4 flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          onClick={dismiss}
          type="button"
        >
          <X aria-hidden="true" className="h-4 w-4" />
        </button>
        <header className="grid gap-2 pr-8 text-left">
          <h2 className="text-xl font-semibold">
            更新日志{version ? ` · v${version}` : ""}
          </h2>
          <p className="text-sm text-muted-foreground">
            最近 10 日的 Buzz 修改记录
          </p>
        </header>
        <div className="min-h-0 overflow-y-auto py-2 pr-2">
          <div className="grid gap-5">
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
        </div>
        <div className="flex justify-end border-t border-border/60 pt-4">
          <Button onClick={dismiss} type="button">
            知道了
          </Button>
        </div>
      </section>
    </div>
  );
}
