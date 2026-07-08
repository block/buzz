import { relaunch } from "@tauri-apps/plugin-process";

import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";

export function RelaunchRequiredScreen() {
  const systemColorScheme = useSystemColorScheme();

  return (
    <div
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-system-color-scheme={systemColorScheme}
      data-testid="relaunch-required"
    >
      <StartupWindowDragRegion />
      <div className="relative flex w-full max-w-[500px] flex-col items-center text-center">
        <h1 className="text-3xl font-semibold tracking-tight">
          Restart Buzz to finish recovery
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          Your identity was restored. Buzz needs to restart so syncing and
          agents run under it.
        </p>
        <Button
          className="mt-8 h-10 w-full max-w-[300px]"
          data-testid="relaunch-app"
          onClick={() => {
            void relaunch();
          }}
          type="button"
        >
          Relaunch Buzz
        </Button>
      </div>
    </div>
  );
}
