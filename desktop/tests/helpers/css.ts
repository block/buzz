import { expect, type Locator } from "@playwright/test";

export async function currentRootFontScale(locator: Locator) {
  const rootFontSize = await locator.evaluate(() =>
    Number.parseFloat(
      window.getComputedStyle(document.documentElement).fontSize,
    ),
  );
  return Number.isFinite(rootFontSize) ? rootFontSize / 16 : 1;
}

export async function expectedScaledPx(
  locator: Locator,
  pxAtDefaultScale: number,
) {
  return pxAtDefaultScale * (await currentRootFontScale(locator));
}

export async function expectCornerRadiusPx(
  locator: Locator,
  expectedRadiusPx: number,
  options: { scaleWithRootFont?: boolean } = {},
) {
  const measurement = await locator.evaluate((element) => {
    const style = window.getComputedStyle(element);
    const rootFontSize = Number.parseFloat(
      window.getComputedStyle(document.documentElement).fontSize,
    );

    const resolveLength = (value: string) => {
      const probe = document.createElement("div");
      for (const sourceStyle of [
        window.getComputedStyle(document.documentElement),
        style,
      ]) {
        for (let i = 0; i < sourceStyle.length; i += 1) {
          const name = sourceStyle.item(i);
          if (name.startsWith("--")) {
            probe.style.setProperty(name, sourceStyle.getPropertyValue(name));
          }
        }
      }
      probe.style.position = "absolute";
      probe.style.visibility = "hidden";
      probe.style.pointerEvents = "none";
      probe.style.width = value;
      document.body.append(probe);
      const resolved = window.getComputedStyle(probe).width;
      probe.remove();
      return resolved;
    };

    const toPx = (value: string): number => {
      const trimmed = value.trim();
      if (/^-?\d+(?:\.\d+)?px$/.test(trimmed)) {
        return Number.parseFloat(trimmed);
      }
      if (/^-?\d+(?:\.\d+)?rem$/.test(trimmed)) {
        return Number.parseFloat(trimmed) * rootFontSize;
      }

      const resolved = resolveLength(trimmed);
      if (resolved !== trimmed) {
        return toPx(resolved);
      }
      return Number.parseFloat(resolved);
    };

    const rawRadius =
      style.borderTopLeftRadius ||
      style.getPropertyValue("border-top-left-radius") ||
      style.borderRadius ||
      style.getPropertyValue("border-radius") ||
      (element instanceof HTMLElement
        ? element.style.borderTopLeftRadius || element.style.borderRadius
        : "");
    const radius = toPx(rawRadius);
    if (!Number.isFinite(radius)) {
      throw new Error(`Could not resolve border radius from "${rawRadius}".`);
    }
    return {
      className: element.getAttribute("class") ?? "",
      radius,
      rawRadius,
      rootFontSize,
    };
  });

  const expected = options.scaleWithRootFont
    ? expectedRadiusPx * (measurement.rootFontSize / 16)
    : expectedRadiusPx;

  expect(
    measurement.radius,
    `Expected ${expected}px corner radius, got ${measurement.rawRadius} on class "${measurement.className}".`,
  ).toBeCloseTo(expected, 0);
}

export async function expectSmoothCorners(
  locator: Locator,
  expectedSmoothing = 0.6,
) {
  await expect
    .poll(async () =>
      locator.evaluate((element, smoothing) => {
        if (!(element instanceof HTMLElement)) {
          return false;
        }

        const clipPath =
          element.style.clipPath ||
          element.style.getPropertyValue("-webkit-clip-path");

        return (
          element.dataset.smoothCorners === "" &&
          element.dataset.smoothCornersSmoothing === String(smoothing) &&
          clipPath.startsWith('path("M ') &&
          clipPath.includes(" a ")
        );
      }, expectedSmoothing),
    )
    .toBe(true);
}
