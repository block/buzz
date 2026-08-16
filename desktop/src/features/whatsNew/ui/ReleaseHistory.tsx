import * as React from "react";

import { getVersion } from "@tauri-apps/api/app";

import { WHATS_NEW_CHANGELOG } from "@/features/whatsNew/changelog";
import {
  coreVersionOf,
  localReleaseRows,
  mergeReleaseTimeline,
} from "@/features/whatsNew/lib/upstreamReleases.mjs";
import { useUpstreamReleasesQuery } from "@/features/whatsNew/useUpstreamReleasesQuery";
import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";

/**
 * Full version-by-version changelog, newest first.
 *
 * This is the canonical home for release history. The launch splash
 * (`WhatsNewModal`) deliberately shows only the running version and points
 * here: a modal that grows by a section per release stops being read, and the
 * history is worth keeping in a place people can go back to rather than one
 * they have to dismiss.
 *
 * Upstream Buzz's own releases are interleaved into the same timeline, capped
 * at the version this fork has caught up to. Because the two are mixed rather
 * than separated, every row is explicitly labelled — without that, "what did
 * my team change" and "what came from upstream" become impossible to tell
 * apart, which is the one real cost of a merged list.
 */
export function ReleaseHistory() {
  const [appVersion, setAppVersion] = React.useState<string | null>(null);

  React.useEffect(() => {
    void getVersion().then(setAppVersion);
  }, []);

  const coreVersion = React.useMemo(
    () => (appVersion ? coreVersionOf(appVersion) : null),
    [appVersion],
  );
  // Supplementary and allowed to fail: offline or rate-limited simply means
  // the fork's own history renders alone.
  const upstreamQuery = useUpstreamReleasesQuery(coreVersion);

  const rows = React.useMemo(
    () =>
      mergeReleaseTimeline(
        localReleaseRows(WHATS_NEW_CHANGELOG),
        upstreamQuery.data ?? [],
      ),
    [upstreamQuery.data],
  );

  return (
    <SettingsOptionGroup title="Release history">
      <div
        className="flex flex-col gap-5 px-3 py-3"
        data-testid="settings-release-history"
      >
        {rows.map((row) => {
          const isCurrent = appVersion === row.version;
          const isUpstream = row.source === "upstream";
          return (
            <div className="flex flex-col gap-1.5" key={row.key}>
              <div className="flex flex-wrap items-center gap-2">
                <p className="text-sm font-medium">
                  {isUpstream ? `Buzz ${row.version}` : row.version}
                </p>
                <span
                  className={
                    isUpstream
                      ? "rounded-full bg-muted px-2 py-0.5 text-2xs font-medium text-muted-foreground"
                      : "rounded-full bg-secondary px-2 py-0.5 text-2xs font-medium text-secondary-foreground"
                  }
                >
                  {isUpstream ? "Upstream Buzz" : "This app"}
                </span>
                {isCurrent ? (
                  <span className="rounded-full bg-primary/15 px-2 py-0.5 text-2xs font-medium text-primary">
                    Current
                  </span>
                ) : null}
                {row.date ? (
                  <span className="text-2xs text-muted-foreground">
                    {row.date}
                  </span>
                ) : null}
              </div>

              {isUpstream ? (
                // Upstream notes are long, Markdown, and written for a
                // different audience. Linking out beats rendering them badly.
                row.url ? (
                  <a
                    className="text-sm text-primary hover:underline"
                    href={row.url}
                    rel="noreferrer"
                    target="_blank"
                  >
                    View release notes on GitHub
                  </a>
                ) : null
              ) : (
                <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">
                  {(row.bullets ?? []).map((bullet) => (
                    <li key={bullet}>{bullet}</li>
                  ))}
                </ul>
              )}
            </div>
          );
        })}
      </div>
    </SettingsOptionGroup>
  );
}
