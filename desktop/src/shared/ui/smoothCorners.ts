import * as React from "react";

export const SMOOTH_CORNER_SMOOTHING = 0.6;

type Corner = "topLeft" | "topRight" | "bottomRight" | "bottomLeft";
type Side = "top" | "right" | "bottom" | "left";

type CornerRadii = Record<Corner, number>;

type NormalizedCorner = {
  radius: number;
  roundingAndSmoothingBudget: number;
};

type NormalizedCorners = Record<Corner, NormalizedCorner>;

type CornerPathParams = {
  a: number;
  b: number;
  c: number;
  d: number;
  p: number;
  arcSectionLength: number;
  cornerRadius: number;
};

type SmoothCornersOptions = {
  enabled?: boolean;
  smoothing?: number;
};

const ADJACENTS_BY_CORNER: Record<
  Corner,
  Array<{ corner: Corner; side: Side }>
> = {
  topLeft: [
    { corner: "topRight", side: "top" },
    { corner: "bottomLeft", side: "left" },
  ],
  topRight: [
    { corner: "topLeft", side: "top" },
    { corner: "bottomRight", side: "right" },
  ],
  bottomRight: [
    { corner: "bottomLeft", side: "bottom" },
    { corner: "topRight", side: "right" },
  ],
  bottomLeft: [
    { corner: "bottomRight", side: "bottom" },
    { corner: "topLeft", side: "left" },
  ],
};

function supportsClipPathPath() {
  return (
    typeof CSS === "undefined" ||
    typeof CSS.supports !== "function" ||
    CSS.supports("clip-path", 'path("M 0 0 L 1 0 L 1 1 L 0 1 Z")') ||
    CSS.supports("-webkit-clip-path", 'path("M 0 0 L 1 0 L 1 1 L 0 1 Z")')
  );
}

function toRadians(degrees: number) {
  return (degrees * Math.PI) / 180;
}

function round(value: number) {
  return Number.isFinite(value) ? value.toFixed(4) : "0.0000";
}

function rounded(strings: TemplateStringsArray, ...values: number[]): string {
  let output = strings[0] ?? "";
  for (let i = 0; i < values.length; i += 1) {
    output += round(values[i]);
    output += strings[i + 1] ?? "";
  }
  return output;
}

function parseRadiusLength(value: string | undefined, axisLength: number) {
  const trimmed = value?.trim();
  if (!trimmed) return 0;

  if (trimmed.endsWith("%")) {
    const percent = Number.parseFloat(trimmed);
    return Number.isFinite(percent) ? (percent / 100) * axisLength : 0;
  }

  const pixels = Number.parseFloat(trimmed);
  return Number.isFinite(pixels) ? Math.max(0, pixels) : 0;
}

function parseCornerRadius(value: string, width: number, height: number) {
  const [horizontal = "0", vertical = horizontal] = value.trim().split(/\s+/);
  return Math.min(
    parseRadiusLength(horizontal, width),
    parseRadiusLength(vertical, height),
  );
}

function getLayoutSize(element: HTMLElement) {
  const style = window.getComputedStyle(element);
  const width = Number.parseFloat(style.width);
  const height = Number.parseFloat(style.height);

  if (!Number.isFinite(width) || !Number.isFinite(height)) {
    return {
      height: element.offsetHeight,
      width: element.offsetWidth,
    };
  }

  if (style.boxSizing === "border-box") {
    return { height, width };
  }

  const paddingX =
    (Number.parseFloat(style.paddingLeft) || 0) +
    (Number.parseFloat(style.paddingRight) || 0);
  const paddingY =
    (Number.parseFloat(style.paddingTop) || 0) +
    (Number.parseFloat(style.paddingBottom) || 0);
  const borderX =
    (Number.parseFloat(style.borderLeftWidth) || 0) +
    (Number.parseFloat(style.borderRightWidth) || 0);
  const borderY =
    (Number.parseFloat(style.borderTopWidth) || 0) +
    (Number.parseFloat(style.borderBottomWidth) || 0);

  return {
    height: height + paddingY + borderY,
    width: width + paddingX + borderX,
  };
}

function readCornerRadii(
  element: HTMLElement,
  width: number,
  height: number,
): CornerRadii {
  const style = window.getComputedStyle(element);
  const inlineStyle = element.style;

  return {
    topLeft: parseCornerRadius(
      style.borderTopLeftRadius ||
        inlineStyle.borderTopLeftRadius ||
        inlineStyle.borderRadius,
      width,
      height,
    ),
    topRight: parseCornerRadius(
      style.borderTopRightRadius ||
        inlineStyle.borderTopRightRadius ||
        inlineStyle.borderRadius,
      width,
      height,
    ),
    bottomRight: parseCornerRadius(
      style.borderBottomRightRadius ||
        inlineStyle.borderBottomRightRadius ||
        inlineStyle.borderRadius,
      width,
      height,
    ),
    bottomLeft: parseCornerRadius(
      style.borderBottomLeftRadius ||
        inlineStyle.borderBottomLeftRadius ||
        inlineStyle.borderRadius,
      width,
      height,
    ),
  };
}

function distributeAndNormalize(
  radii: CornerRadii,
  width: number,
  height: number,
): NormalizedCorners {
  const radiusMap: CornerRadii = { ...radii };
  const budgetMap: Record<Corner, number> = {
    topLeft: -1,
    topRight: -1,
    bottomRight: -1,
    bottomLeft: -1,
  };

  (Object.entries(radiusMap) as Array<[Corner, number]>)
    .sort(([, first], [, second]) => second - first)
    .forEach(([corner, radius]) => {
      const budget = Math.min(
        ...ADJACENTS_BY_CORNER[corner].map((adjacent) => {
          const adjacentRadius = radiusMap[adjacent.corner];
          if (radius === 0 && adjacentRadius === 0) {
            return 0;
          }

          const adjacentBudget = budgetMap[adjacent.corner];
          const sideLength =
            adjacent.side === "top" || adjacent.side === "bottom"
              ? width
              : height;

          if (adjacentBudget >= 0) {
            return sideLength - adjacentBudget;
          }

          return (radius / (radius + adjacentRadius)) * sideLength;
        }),
      );

      budgetMap[corner] = budget;
      radiusMap[corner] = Math.min(radius, budget);
    });

  return {
    topLeft: {
      radius: radiusMap.topLeft,
      roundingAndSmoothingBudget: budgetMap.topLeft,
    },
    topRight: {
      radius: radiusMap.topRight,
      roundingAndSmoothingBudget: budgetMap.topRight,
    },
    bottomRight: {
      radius: radiusMap.bottomRight,
      roundingAndSmoothingBudget: budgetMap.bottomRight,
    },
    bottomLeft: {
      radius: radiusMap.bottomLeft,
      roundingAndSmoothingBudget: budgetMap.bottomLeft,
    },
  };
}

function getPathParamsForCorner({
  cornerRadius,
  cornerSmoothing,
  roundingAndSmoothingBudget,
}: {
  cornerRadius: number;
  cornerSmoothing: number;
  roundingAndSmoothingBudget: number;
}): CornerPathParams {
  if (cornerRadius <= 0) {
    return {
      a: 0,
      b: 0,
      c: 0,
      d: 0,
      p: 0,
      arcSectionLength: 0,
      cornerRadius: 0,
    };
  }

  let p = (1 + cornerSmoothing) * cornerRadius;
  const arcMeasure = 90 * (1 - cornerSmoothing);
  const arcSectionLength =
    Math.sin(toRadians(arcMeasure / 2)) * cornerRadius * Math.sqrt(2);
  const angleAlpha = (90 - arcMeasure) / 2;
  const p3ToP4Distance = cornerRadius * Math.tan(toRadians(angleAlpha / 2));
  const angleBeta = 45 * cornerSmoothing;
  const c = p3ToP4Distance * Math.cos(toRadians(angleBeta));
  const d = c * Math.tan(toRadians(angleBeta));

  let b = (p - arcSectionLength - c - d) / 3;
  let a = 2 * b;

  if (p > roundingAndSmoothingBudget) {
    const p1ToP3MaxDistance =
      roundingAndSmoothingBudget - d - arcSectionLength - c;

    const minA = p1ToP3MaxDistance / 6;
    const maxB = p1ToP3MaxDistance - minA;

    b = Math.min(b, maxB);
    a = p1ToP3MaxDistance - b;
    p = Math.min(p, roundingAndSmoothingBudget);
  }

  return { a, b, c, d, p, arcSectionLength, cornerRadius };
}

function buildCorner(
  corner: NormalizedCorner,
  smoothing: number,
): { p: number; pathSegment: (corner: Corner) => string } {
  const params = getPathParamsForCorner({
    cornerRadius: corner.radius,
    cornerSmoothing: smoothing,
    roundingAndSmoothingBudget: corner.roundingAndSmoothingBudget,
  });

  if (params.cornerRadius <= 0) {
    return { p: 0, pathSegment: () => "" };
  }

  return {
    p: params.p,
    pathSegment: (name) => {
      switch (name) {
        case "topRight":
          return drawTopRightPath(params);
        case "bottomRight":
          return drawBottomRightPath(params);
        case "bottomLeft":
          return drawBottomLeftPath(params);
        case "topLeft":
          return drawTopLeftPath(params);
      }
    },
  };
}

function drawTopRightPath({
  cornerRadius,
  a,
  b,
  c,
  d,
  arcSectionLength,
}: CornerPathParams): string {
  return rounded`c ${a} 0 ${a + b} 0 ${a + b + c} ${d} a ${cornerRadius} ${cornerRadius} 0 0 1 ${arcSectionLength} ${arcSectionLength} c ${d} ${c} ${d} ${b + c} ${d} ${a + b + c}`;
}

function drawBottomRightPath({
  cornerRadius,
  a,
  b,
  c,
  d,
  arcSectionLength,
}: CornerPathParams): string {
  return rounded`c 0 ${a} 0 ${a + b} ${-d} ${a + b + c} a ${cornerRadius} ${cornerRadius} 0 0 1 ${-arcSectionLength} ${arcSectionLength} c ${-c} ${d} ${-(b + c)} ${d} ${-(a + b + c)} ${d}`;
}

function drawBottomLeftPath({
  cornerRadius,
  a,
  b,
  c,
  d,
  arcSectionLength,
}: CornerPathParams): string {
  return rounded`c ${-a} 0 ${-(a + b)} 0 ${-(a + b + c)} ${-d} a ${cornerRadius} ${cornerRadius} 0 0 1 ${-arcSectionLength} ${-arcSectionLength} c ${-d} ${-c} ${-d} ${-(b + c)} ${-d} ${-(a + b + c)}`;
}

function drawTopLeftPath({
  cornerRadius,
  a,
  b,
  c,
  d,
  arcSectionLength,
}: CornerPathParams): string {
  return rounded`c 0 ${-a} 0 ${-(a + b)} ${d} ${-(a + b + c)} a ${cornerRadius} ${cornerRadius} 0 0 1 ${arcSectionLength} ${-arcSectionLength} c ${c} ${-d} ${b + c} ${-d} ${a + b + c} ${-d}`;
}

export function generateSmoothCornerPath(
  width: number,
  height: number,
  radii: CornerRadii,
  smoothing = SMOOTH_CORNER_SMOOTHING,
) {
  if (width <= 0 || height <= 0) {
    return "M 0 0 H 0 V 0 H 0 Z";
  }

  const normalized = distributeAndNormalize(radii, width, height);
  const topLeft = buildCorner(normalized.topLeft, smoothing);
  const topRight = buildCorner(normalized.topRight, smoothing);
  const bottomRight = buildCorner(normalized.bottomRight, smoothing);
  const bottomLeft = buildCorner(normalized.bottomLeft, smoothing);

  const seg = (segment: string) => (segment.length > 0 ? ` ${segment}` : "");

  return (
    `M ${round(topLeft.p)} 0` +
    ` L ${round(width - topRight.p)} 0` +
    seg(topRight.pathSegment("topRight")) +
    ` L ${round(width)} ${round(bottomRight.p)}` +
    ` L ${round(width)} ${round(height - bottomRight.p)}` +
    seg(bottomRight.pathSegment("bottomRight")) +
    ` L ${round(width - bottomLeft.p)} ${round(height)}` +
    ` L ${round(bottomLeft.p)} ${round(height)}` +
    seg(bottomLeft.pathSegment("bottomLeft")) +
    ` L 0 ${round(height - topLeft.p)}` +
    ` L 0 ${round(topLeft.p)}` +
    seg(topLeft.pathSegment("topLeft")) +
    " Z"
  );
}

export function generateSmoothCornerClipPath(
  width: number,
  height: number,
  radii: CornerRadii,
  smoothing = SMOOTH_CORNER_SMOOTHING,
) {
  return `path("${generateSmoothCornerPath(width, height, radii, smoothing)}")`;
}

export function useSmoothCorners<T extends HTMLElement>(
  ref: React.RefObject<T | null>,
  options: SmoothCornersOptions = {},
) {
  const enabled = options.enabled ?? true;
  const smoothing = options.smoothing ?? SMOOTH_CORNER_SMOOTHING;

  React.useLayoutEffect(() => {
    if (!enabled || typeof window === "undefined") return;

    const element = ref.current;
    if (!element) return;
    if (!supportsClipPathPath()) return;

    const savedClipPath = element.style.clipPath;
    const savedWebkitClipPath =
      element.style.getPropertyValue("-webkit-clip-path");
    let animationFrame = 0;
    let lastClipPath = "";

    const sync = () => {
      const { width, height } = getLayoutSize(element);
      if (width <= 0 || height <= 0) return;

      const clipPath = generateSmoothCornerClipPath(
        width,
        height,
        readCornerRadii(element, width, height),
        smoothing,
      );

      if (clipPath === lastClipPath) return;

      element.style.clipPath = clipPath;
      element.style.setProperty("-webkit-clip-path", clipPath);
      element.dataset.smoothCorners = "";
      element.dataset.smoothCornersSmoothing = String(smoothing);
      lastClipPath = clipPath;
    };

    const scheduleSync = () => {
      if (animationFrame) return;
      animationFrame = window.requestAnimationFrame(() => {
        animationFrame = 0;
        sync();
      });
    };

    sync();

    const mutationObserver = new MutationObserver(scheduleSync);
    mutationObserver.observe(element, {
      attributeFilter: ["class", "style"],
      attributes: true,
    });

    let resizeObserver: ResizeObserver | undefined;
    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(scheduleSync);
      resizeObserver.observe(element);
    } else {
      window.addEventListener("resize", scheduleSync);
    }

    return () => {
      if (animationFrame) {
        window.cancelAnimationFrame(animationFrame);
      }
      mutationObserver.disconnect();
      resizeObserver?.disconnect();
      if (!resizeObserver) {
        window.removeEventListener("resize", scheduleSync);
      }
      element.style.clipPath = savedClipPath;
      if (savedWebkitClipPath) {
        element.style.setProperty("-webkit-clip-path", savedWebkitClipPath);
      } else {
        element.style.removeProperty("-webkit-clip-path");
      }
      delete element.dataset.smoothCorners;
      delete element.dataset.smoothCornersSmoothing;
    };
  }, [enabled, ref, smoothing]);
}
