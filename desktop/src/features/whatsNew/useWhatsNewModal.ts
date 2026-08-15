import * as React from "react";

import { getVersion } from "@tauri-apps/api/app";

import {
  changelogEntryForVersion,
  type ChangelogEntry,
} from "@/features/whatsNew/changelog";
import {
  readLastSeenChangelogVersion,
  writeLastSeenChangelogVersion,
} from "@/features/whatsNew/whatsNewStorage";

type UseWhatsNewModalResult = {
  /** The running version's changelog entry, or null if it has none. */
  entry: ChangelogEntry | null;
  isOpen: boolean;
  onDismiss: () => void;
};

/**
 * Decides whether the "what's new" splash should be showing: it opens once per
 * real app version change and stays closed once dismissed for that version.
 *
 * Reads the actual running version via `getVersion()` (async, hence the effect
 * below) rather than a separate build label — there's no longer a distinct
 * "dev build" identity to key off. The version string is matched against the
 * changelog whole, never parsed for a number: deriving the entry from the
 * trailing `-N` is what silently disabled this splash when the fork renumbered
 * from `0.5.5-5` to `0.5.14-0`.
 *
 * A version with no changelog entry (a dev build, or a release with no
 * user-facing change) simply doesn't open the splash — there is nothing to
 * announce, and the full history is in Settings → Updates either way.
 */
export function useWhatsNewModal(): UseWhatsNewModalResult {
  const [appVersion, setAppVersion] = React.useState<string | null>(null);

  React.useEffect(() => {
    void getVersion().then(setAppVersion);
  }, []);

  const entry = React.useMemo(
    () => changelogEntryForVersion(appVersion),
    [appVersion],
  );

  const [isOpen, setIsOpen] = React.useState(false);
  const hasDecidedRef = React.useRef(false);

  React.useEffect(() => {
    if (hasDecidedRef.current) return;
    if (appVersion === null || entry === null) return;
    hasDecidedRef.current = true;
    setIsOpen(readLastSeenChangelogVersion() !== appVersion);
  }, [appVersion, entry]);

  const onDismiss = React.useCallback(() => {
    if (appVersion) {
      writeLastSeenChangelogVersion(appVersion);
    }
    setIsOpen(false);
  }, [appVersion]);

  return { entry, isOpen, onDismiss };
}
