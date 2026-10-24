import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import React from "react";
import { act, cleanup, render } from "@testing-library/react";
import { setAppLanguage } from "../../../i18n.ts";
import { uiT } from "../../../shared/i18n/uiTranslation.ts";
import { NotificationSettingsCard } from "./NotificationSettingsCard.tsx";
import {
  DEFAULT_SLOT_SOUNDS,
  DEFAULT_SLOT_ALERTS_ENABLED,
  SLOT_LABELS,
  SLOT_DESCRIPTIONS,
} from "../../notifications/lib/sound.ts";

afterEach(async () => {
  cleanup();
  await setAppLanguage("en");
  localStorage.clear();
});
test("enabled alert labels change language without changing sound or slot identifiers", async () => {
  const settings = {
    desktopEnabled: true,
    homeBadgeEnabled: true,
    notifyWhileViewing: false,
    sounds: { ...DEFAULT_SLOT_SOUNDS },
    slotAlertsEnabled: { ...DEFAULT_SLOT_ALERTS_ENABLED },
    slotAlertsSnapshot: null,
  };
  const before = structuredClone(settings);
  const noop = () => {};
  const screen = render(
    React.createElement(NotificationSettingsCard, {
      isUpdatingDesktopNotifications: false,
      notificationErrorMessage: null,
      notificationPermission: "granted",
      notificationSettings: settings,
      onSetDesktopNotificationsEnabled: async () => true,
      onSetAllSlotAlertsEnabled: noop,
      onSetHomeBadgeEnabled: noop,
      onSetSlotAlertsEnabled: noop,
      onSetNotifyWhileViewing: noop,
      onSetSoundForSlot: noop,
    }),
  );
  for (const language of ["en", "ko", "en"]) {
    await act(() => setAppLanguage(language));
    for (const slot of ["dm", "mention", "thread_reply", "needs_action"]) {
      assert.ok(screen.container.textContent.includes(uiT(SLOT_LABELS[slot])));
      assert.ok(
        screen.container.textContent.includes(uiT(SLOT_DESCRIPTIONS[slot])),
      );
    }
    assert.equal(
      screen.getByTestId("notifications-desktop-state").textContent,
      uiT("On"),
    );
    assert.deepEqual(settings, before);
  }
});
