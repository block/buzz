import * as React from "react";

import { getVersion } from "@tauri-apps/api/app";

import {
  changelogEntriesUpToVersion,
  parseReleaseNumber,
  type ChangelogEntry,
} from "@/features/whatsNew/changelog";
import {
  readLastSeenChangelogVersion,
  writeLastSeenChangelogVersion,
} from "@/features/whatsNew/whatsNewStorage";

type UseWhatsNewModalResult = {
  entries: ChangelogEntry[];
  isOpen: boolean;
  onDismiss: () => void;
};

/**
 * Decides whether the "what's new" splash should be showing: it opens once
 * per real app version change (including the very first run, where nothing
 * has been recorded yet) and stays closed once dismissed for that version.
 *
 * Reads the actual running version via `getVersion()` (async, hence the
 * effect below) rather than a separate build label — there's no longer a
 * distinct "dev build" identity to key off. If the version can't be parsed
 * for a trailing `-N` (shouldn't happen for a real k2alpha build), the
 * splash simply never opens.
 */
export function useWhatsNewModal(): UseWhatsNewModalResult {
  const [appVersion, setAppVersion] = React.useState<string | null>(null);

  React.useEffect(() => {
    void getVersion().then(setAppVersion);
  }, []);

  const releaseNumber = React.useMemo(
    () => parseReleaseNumber(appVersion),
    [appVersion],
  );

  const entries = React.useMemo(
    () => changelogEntriesUpToVersion(releaseNumber),
    [releaseNumber],
  );

  const [isOpen, setIsOpen] = React.useState(false);
  const hasDecidedRef = React.useRef(false);

  React.useEffect(() => {
    if (hasDecidedRef.current) return;
    if (appVersion === null || entries.length === 0) return;
    hasDecidedRef.current = true;
    setIsOpen(readLastSeenChangelogVersion() !== appVersion);
  }, [appVersion, entries.length]);

  const onDismiss = React.useCallback(() => {
    if (appVersion) {
      writeLastSeenChangelogVersion(appVersion);
    }
    setIsOpen(false);
  }, [appVersion]);

  return { entries, isOpen, onDismiss };
}
