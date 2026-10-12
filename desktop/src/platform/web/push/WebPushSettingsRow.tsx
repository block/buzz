// Settings → Notifications row for Web Push (web build only, rendered only when the push flag is on).
import * as React from "react";

import { SettingsOptionRow } from "@/features/settings/ui/SettingsOptionGroup";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import { readPushEnabled, writePushEnabled } from "./config";
import { disablePush, enablePushFromTap, pushUiState } from "./client";

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
