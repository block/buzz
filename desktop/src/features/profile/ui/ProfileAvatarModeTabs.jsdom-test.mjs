import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import React from "react";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { setAppLanguage } from "../../../i18n.ts";
import { ProfileAvatarModeTabs } from "./ProfileAvatarModeTabs.tsx";

afterEach(async () => {
  cleanup();
  await setAppLanguage("en");
  localStorage.clear();
});

for (const presentation of ["onboarding-inline", "onboarding-modal"]) {
  test(`${presentation} avatar labels update without changing the selected mode ID`, async () => {
    await setAppLanguage("en");
    const changes = [];
    function Screen() {
      const [mode, setMode] = React.useState("image");
      return React.createElement(ProfileAvatarModeTabs, {
        mode,
        presentation,
        disabled: false,
        onModeChange: (value) => {
          changes.push(value);
          setMode(value);
        },
      });
    }
    const screen = render(React.createElement(Screen));
    const role = presentation === "onboarding-inline" ? "button" : "tab";
    const image = screen.getByRole(role, { name: "Image", exact: true });
    await act(() => setAppLanguage("ko"));
    assert.equal(
      screen.getByRole(role, { name: "이미지", exact: true }),
      image,
    );
    const emoji = screen.getByRole(role, { name: "이모지", exact: true });
    if (role === "tab")
      fireEvent.mouseDown(emoji, { button: 0, ctrlKey: false });
    else fireEvent.click(emoji);
    assert.deepEqual(changes, ["emoji"]);
    await act(() => setAppLanguage("en"));
    const selected = screen.getByRole(role, { name: "Emoji", exact: true });
    assert.equal(
      selected.getAttribute(role === "tab" ? "aria-selected" : "aria-pressed"),
      "true",
    );
    assert.deepEqual(changes, ["emoji"]);
  });
}
