// Settings → Notifications row for Web Push (web build only, rendered only when the push flag is on).
import * as React from "react";

import { SettingsOptionRow } from "@/features/settings/ui/SettingsOptionGroup";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import { readPushEnabled, writePushEnabled } from "./config";
import { disablePush, enablePushFromTap, pushUiState } from "./client";
import { diag, subscribeDiag } from "./diag";
import { readPushEnv } from "./env";

const ago = (t?: number) =>
  t
    ? `${new Date(t).toLocaleTimeString()} (${Math.round((Date.now() - t) / 1000)}s ago)`
    : "never";

/** One-line-per-check push status (no secrets) so a failing step is visible on the device itself. */
function PushStatus() {
  const [, bump] = React.useReducer((n: number) => n + 1, 0);
  const [sw, setSw] = React.useState<{ reg: boolean; sub: boolean } | null>(
    null,
  );
  React.useEffect(() => subscribeDiag(bump), []);
  React.useEffect(() => {
    let alive = true;
    const probe = async () => {
      const reg =
        (await navigator.serviceWorker
          ?.getRegistration()
          .catch(() => undefined)) ?? null;
      const sub = reg
        ? await reg.pushManager?.getSubscription().catch(() => null)
        : null;
      if (alive) setSw({ reg: !!reg, sub: !!sub });
    };
    void probe();
    const t = window.setInterval(() => {
      void probe();
      bump();
    }, 3000);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, []);
  const env = readPushEnv();
  const d = diag();
  const lines = [
    `permission: ${env.permission} · standalone: ${env.standalone ? "yes" : "no"}`,
    `service worker: ${sw ? (sw.reg ? "registered" : "missing") : "…"} · subscription: ${sw ? (sw.sub ? "present" : "none") : "…"}`,
    `relay info: ${d.relayInfo ?? "not checked yet"}`,
    `gateway: ${d.gateway ?? "not contacted yet"}`,
    `last lease: ${ago(d.leaseAt)} — ${d.lease ?? "not sent yet"}`,
    `pushes received while open: ${d.pushes ?? 0}${d.lastPushAt ? ` (last ${ago(d.lastPushAt)})` : ""}`,
    `build: ${import.meta.env.VITE_BUZZ_WEB_BUILD_ID || "dev"}`,
  ];
  return (
    <pre
      className="mt-2 whitespace-pre-wrap break-words text-xs text-muted-foreground/70"
      data-testid="web-push-status"
    >
      {lines.join("\n")}
    </pre>
  );
}

const HINT: Record<string, string> = {
  ready:
    "Mentions, DMs, thread replies and approvals arrive even when the app is closed.",
  "needs-tap": "Tap Enable and allow notifications when your browser asks.",
  "ios-install":
    "On iPhone and iPad, add Hula Buzz to your Home Screen and open it from there to get notifications.",
  denied:
    "Notifications are blocked. Allow them in your browser or iOS Settings → Notifications, then come back.",
  unsupported: "This browser can't receive push notifications.",
};

export default function WebPushSettingsRow() {
  const [on, setOn] = React.useState(readPushEnabled);
  const [state, setState] = React.useState(pushUiState);
  return (
    <SettingsOptionRow>
      <div className="min-w-0">
        <label className="text-sm font-medium" htmlFor="web-push-switch">
          Push notifications
        </label>
        <p
          className="text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
          data-testid="web-push-hint"
        >
          {on ? HINT[state] : "Off on this device."}
        </p>
        <PushStatus />
      </div>
      <div className="flex items-center gap-2">
        {on && state === "needs-tap" ? (
          <Button
            data-testid="web-push-enable"
            onClick={() => void enablePushFromTap().then(setState)}
            type="button"
            variant="outline"
          >
            Enable
          </Button>
        ) : null}
        <Switch
          checked={on}
          data-testid="web-push-toggle"
          id="web-push-switch"
          onCheckedChange={(checked) => {
            setOn(checked);
            writePushEnabled(checked);
            if (!checked) void disablePush();
            setState(pushUiState());
          }}
        />
      </div>
    </SettingsOptionRow>
  );
}
