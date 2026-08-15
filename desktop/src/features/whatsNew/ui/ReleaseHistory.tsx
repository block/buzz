import * as React from "react";

import { getVersion } from "@tauri-apps/api/app";

import { WHATS_NEW_CHANGELOG } from "@/features/whatsNew/changelog";
import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";

/**
 * Full version-by-version changelog, newest first.
 *
 * This is the canonical home for release history. The launch splash
 * (`WhatsNewModal`) deliberately shows only the running version and points
 * here: a modal that grows by a section per release stops being read, and the
 * history is worth keeping in a place people can go back to rather than one
 * they have to dismiss.
 */
export function ReleaseHistory() {
  const [appVersion, setAppVersion] = React.useState<string | null>(null);

  React.useEffect(() => {
    void getVersion().then(setAppVersion);
  }, []);

  // Newest first — the array is maintained oldest-first so that position can
  // act as release order everywhere else.
  const entries = React.useMemo(() => [...WHATS_NEW_CHANGELOG].reverse(), []);

  return (
    <SettingsOptionGroup title="Release history">
      <div
        className="flex flex-col gap-5 px-3 py-3"
        data-testid="settings-release-history"
      >
        {entries.map((entry) => {
          const isCurrent = appVersion === entry.version;
          return (
            <div className="flex flex-col gap-1.5" key={entry.version}>
              <div className="flex items-center gap-2">
                <p className="text-sm font-medium">{entry.version}</p>
                {isCurrent ? (
                  <span className="rounded-full bg-primary/15 px-2 py-0.5 text-2xs font-medium text-primary">
                    Current
                  </span>
                ) : null}
              </div>
              <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">
                {entry.bullets.map((bullet) => (
                  <li key={bullet}>{bullet}</li>
                ))}
              </ul>
            </div>
          );
        })}
      </div>
    </SettingsOptionGroup>
  );
}
