import { expect, test, type Locator } from "@playwright/test";

async function colors(button: Locator) {
  return button.evaluate((element) => {
    const style = getComputedStyle(element);
    const probe = document.createElement("span");
    probe.style.color = "var(--text-disabled)";
    probe.style.backgroundColor = "var(--neutral-3)";
    element.appendChild(probe);
    const expected = getComputedStyle(probe);
    const result = {
      color: style.color,
      background: style.backgroundColor,
      disabledColor: expected.color,
      disabledBackground: expected.backgroundColor,
    };
    probe.remove();
    return result;
  });
}

for (const dark of [false, true]) {
  test(`disabled round buttons lose active accent in ${dark ? "dark" : "light"} mode`, async ({
    page,
  }) => {
    await page.goto("/design/components/icon-button");
    if (dark) await page.getByRole("button", { name: "Use dark mode" }).click();
    for (const variant of ["tint", "solid"]) {
      const button = page.getByRole("button", {
        name: `Disabled round ${variant}`,
      });
      await expect(button).toBeDisabled();
      await expect
        .poll(async () => {
          const c = await colors(button);
          return (
            c.color === c.disabledColor && c.background === c.disabledBackground
          );
        })
        .toBe(true);
      const before = await colors(button);
      expect(before.color).toBe(before.disabledColor);
      expect(before.background).toBe(before.disabledBackground);
      await button.hover({ force: true });
      const after = await colors(button);
      expect(after.color).toBe(before.color);
      expect(after.background).toBe(before.background);
    }
    await page.goto("/design/components/composer");
    const unavailable = page.getByRole("region", {
      name: "Unavailable",
      exact: true,
    });
    const send = unavailable.getByRole("button", { name: "Send message" });
    await expect(send).toBeDisabled();
    const actual = await colors(send);
    expect(actual.color).toBe(actual.disabledColor);
    expect(actual.background).toBe(actual.disabledBackground);
    const playground = page.getByRole("region", {
      name: "Composer playground",
    });
    await playground.getByRole("textbox").fill("Ready to send");
    const enabled = playground.getByRole("button", { name: "Send message" });
    await expect(enabled).toBeEnabled();
    await expect
      .poll(async () => (await colors(enabled)).color)
      .not.toBe(actual.disabledColor);
  });
}
