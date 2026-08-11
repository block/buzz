import * as React from "react";

import { DEV_BUILD_LABEL } from "@/shared/lib/devBuildLabel";
import {
  changelogEntriesUpToLabel,
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
 * per `DEV_BUILD_LABEL` change (including the very first run, where nothing
 * has been recorded yet) and stays closed once dismissed for that label.
 *
 * `DEV_BUILD_LABEL === null` (a real release build) disables the splash
 * entirely — there's no version identity to key the "seen" flag off.
 */
export function useWhatsNewModal(): UseWhatsNewModalResult {
  const entries = React.useMemo(
    () => changelogEntriesUpToLabel(DEV_BUILD_LABEL),
    [],
  );

  const [isOpen, setIsOpen] = React.useState(() => {
    if (!DEV_BUILD_LABEL || entries.length === 0) return false;
    return readLastSeenChangelogVersion() !== DEV_BUILD_LABEL;
  });

  const onDismiss = React.useCallback(() => {
    if (DEV_BUILD_LABEL) {
      writeLastSeenChangelogVersion(DEV_BUILD_LABEL);
    }
    setIsOpen(false);
  }, []);

  return { entries, isOpen, onDismiss };
}
