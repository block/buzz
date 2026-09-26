import { buildBrowserSessionCard } from "@/features/browsers/lib/addBrowserSession";
import type { PlaygroundCard } from "@/features/playground/lib/types";
import type { PinnedSiteDraft } from "@/features/pinned-sites/lib/types";
import { pinRunbookRef, sidRunbookRef } from "@/features/site-runbook/lib/keys";
import { setSiteRunbook } from "@/features/site-runbook/lib/store";

import { shareRunbookToSiteRunbook } from "./serialize";
import type { HulaBrowserShareV1 } from "./types";

export type ImportBrowserAs = "session" | "pin";

export type AppliedBrowserShareSession = {
  kind: "session";
  card: PlaygroundCard;
};

export type AppliedBrowserSharePin = {
  kind: "pin";
  draft: PinnedSiteDraft;
  /** Call after savePin resolves so the runbook keys to the new pin id. */
  writeRunbookForPinId: (pinId: string) => void;
};

/** Build playground card + write sid runbook. Caller opens the session. */
export function applyBrowserShareAsSession(
  share: HulaBrowserShareV1,
): AppliedBrowserShareSession | null {
  const card = buildBrowserSessionCard({
    url: share.url,
    name: share.title,
  });
  if (!card) return null;
  setSiteRunbook(
    sidRunbookRef(card.sid),
    shareRunbookToSiteRunbook(share.runbook),
  );
  return { kind: "session", card };
}

/** Build a personal pin draft; write runbook after the pin is saved. */
export function prepareBrowserShareAsPin(
  share: HulaBrowserShareV1,
): AppliedBrowserSharePin | null {
  const card = buildBrowserSessionCard({
    url: share.url,
    name: share.title,
  });
  if (!card) return null;
  const draft: PinnedSiteDraft = {
    name: card.name,
    url: card.url,
    icon: "globe",
    pollForChanges: false,
    openMatchingLinks: true,
    community: false,
  };
  return {
    kind: "pin",
    draft,
    writeRunbookForPinId: (pinId: string) => {
      setSiteRunbook(
        pinRunbookRef(pinId),
        shareRunbookToSiteRunbook(share.runbook),
      );
    },
  };
}
