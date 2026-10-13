import { onResume } from "@/shared/lib/resumeSignals";
// Top nudge to turn on push, shown in the shared status host (statusPills.ts: one look, placement and timing).
import {
  bannerDismissed,
  dismissBanner,
  readPushEnabled,
  WEB_PUSH_CHANGE_EVENT,
  WEB_PUSH_FLAG,
} from "./config";
import {
  enablePushFromTap,
  ensurePushSubscribed,
  installPushClickRouting,
  pushUiState,
} from "./client";
import { effectivePermission, type PushUiState } from "./env";
import { clearStatus, showStatus } from "@/shared/ui/statusPills";

const COPY = {
  "ios-install": {
    title: "Get notifications",
    description:
      "Tap Share, then Add to Home Screen, open HulaBuzz from your Home Screen and tap Enable.",
  },
  "needs-tap": {
    title: "Turn on notifications",
    description: "Mentions, DMs and approvals reach you even when the app is closed.",
  },
  denied: {
    title: "Notifications are off",
    description: "On iPhone: Settings → Notifications → HulaBuzz → Allow Notifications.",
  },
} as const;

type BannerState = keyof typeof COPY;
const ID = "web-push-banner";

function hide() {
  clearStatus(ID);
}

/**
 * Root cause of the banner staying up after Allow (iPhone, installed PWA): the Enable handler hid the banner only after
 * `enablePushFromTap()` resolved, and that awaited the whole subscribe chain (relay info, pushManager.subscribe, NIP-98
 * gateway enroll and the NIP-44 + signed kind:30350 lease), which waits on the remote signer and can take minutes or
 * never settle. iOS standalone also doesn't fire Permissions-API change events, and `Notification.permission` can lag
 * at "default" right after the grant, so no recheck caught it either. Now the OS answer alone decides: granted (live or
 * remembered from requestPermission) hides at once, and the subscription is created and synced in the background.
 */
function permissionGranted(): boolean {
  return (
    typeof Notification !== "undefined" &&
    effectivePermission(Notification.permission) === "granted"
  );
}

let shownState: BannerState | null = null;

function render() {
  // Hidden whenever notifications are on (OS permission granted), push is turned off, or the user hid the nudge.
  if (!readPushEnabled() || bannerDismissed() || permissionGranted()) {
    shownState = null;
    hide();
    return;
  }
  const state = pushUiState() as PushUiState;
  const show: BannerState | null =
    state === "ios-install" || state === "needs-tap" || state === "denied"
      ? state
      : null;
  if (!show) {
    shownState = null;
    hide();
    return;
  }
  if (shownState === show) return;
  shownState = show;
  const node = showStatus(ID, {
    tone: "action",
    title: COPY[show].title,
    description: COPY[show].description,
    testId: "web-push-banner",
    action:
      show === "needs-tap"
        ? {
            label: "Enable",
            testId: "web-push-enable",
            onClick: () => void onEnable(),
          }
        : undefined,
    onHide: () => {
      dismissBanner();
      shownState = null;
      hide();
    },
  });
  node?.setAttribute("data-web-push-banner", show);
}

async function onEnable() {
  const btn = document.querySelector<HTMLButtonElement>('[data-testid="web-push-enable"]');
  if (btn) btn.disabled = true;
  const next = await enablePushFromTap().catch(() => pushUiState());
  if (next === "ready" || permissionGranted()) {
    shownState = null;
    hide(); // the OS said yes: hide now; the subscription syncs in the background (enablePushFromTap)
  } else if (btn) btn.disabled = false;
  window.dispatchEvent(new CustomEvent(WEB_PUSH_CHANGE_EVENT));
  // iOS can flip Notification.permission a moment after the dialog closes: recheck for a few seconds.
  for (const ms of [500, 1500, 4000]) setTimeout(render, ms);
  render();
}

function watchPermissionChanges() {
  try {
    const perms = navigator.permissions;
    if (!perms?.query) return;
    void perms
      .query({ name: "notifications" as PermissionName })
      .then((status) => {
        status.onchange = () => {
          render();
          if (permissionGranted()) {
            void ensurePushSubscribed().catch(() => false);
          }
        };
      })
      .catch(() => undefined);
  } catch {
    /* Permissions API missing or notifications name unsupported */
  }
}

/** Boot hook: no-op unless the push service flag is set at build time. */
export function startWebPush() {
  if (!WEB_PUSH_FLAG) return;
  installPushClickRouting();
  watchPermissionChanges();
  // Re-send the subscription + lease whenever the app opens or comes back (the relay may have dropped it).
  // After a failure, wait 10 minutes before asking the signer again so a rejecting relay can't prompt on every resume.
  let failedAt = 0;
  const sync = () => {
    if (failedAt && Date.now() - failedAt < 10 * 60_000) return;
    void ensurePushSubscribed()
      .then(() => {
        failedAt = 0;
        render(); // hide once subscribe succeeds after a grant
      })
      .catch(() => {
        failedAt = Date.now();
      });
  };
  sync();
  render();
  setTimeout(render, 1500);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") render();
  });
  window.addEventListener(WEB_PUSH_CHANGE_EVENT, () => {
    render();
    sync();
  });
  // visible / pageshow / focus / online: re-check permission + subscription and refresh the lease
  onResume(() => {
    render();
    sync();
  });
}
