import * as React from "react";

export type EmojiAvatarDescriptor = {
  color: string;
  emoji: string;
};

export const AVATAR_COLORS = [
  "#FFFFFF",
  "#FFF4CC",
  "#FFE75C",
  "#FFB84D",
  "#FF8652",
  "#F6534F",
  "#FF6B9A",
  "#FB60C4",
  "#D66BFF",
  "#B141FF",
  "#7C5CFF",
  "#476CFF",
  "#3399FF",
  "#63C6F2",
  "#41EBC1",
  "#2ED3A2",
  "#73EF75",
  "#9FE870",
  "#C7D36F",
  "#CCCCCC",
  "#8A8F98",
  "#4B5563",
  "#000000",
];
export const CUSTOM_AVATAR_COLOR_SWATCH = "custom";
export const AVATAR_COLOR_SWATCHES = [
  ...AVATAR_COLORS,
  CUSTOM_AVATAR_COLOR_SWATCH,
] as const;
export type AvatarColorSwatch = (typeof AVATAR_COLOR_SWATCHES)[number];

export const DEFAULT_EMOJI_AVATAR_COLOR = "#FFFFFF";
export const DEFAULT_CUSTOM_HUE = 210;
export const DEFAULT_CUSTOM_SATURATION = 76;
export const DEFAULT_CUSTOM_VALUE = 92;
export const CUSTOM_COLOR_GRID_COLUMNS = 15;
export const CUSTOM_COLOR_GRID_ROWS = 8;
export const CUSTOM_COLOR_GRID_HORIZONTAL_INSET = 24;
export const CUSTOM_COLOR_GRID_VERTICAL_INSET = 24;
export const CUSTOM_HUE_SCRUBBER_INSET = 20;
export const EMOJI_MART_CATEGORIES = [
  "people",
  "nature",
  "foods",
  "activity",
  "places",
  "objects",
  "symbols",
  "flags",
];

const EMOJI_AVATAR_DATA_URL_PREFIX = "data:image/svg+xml,";
const EMOJI_AVATAR_FONT_SIZE = 258;

const EMOJI_MART_SHADOW_CSS = `
  :host {
    display: block;
    height: 100%;
    max-height: 100%;
    min-height: 0;
    overflow: hidden;
    width: 100%;
  }

  #root {
    --padding: 16px;
    --sidebar-width: 0px;
    display: flex;
    flex-direction: column;
    height: 100%;
    max-height: 100%;
    min-height: 0;
    overflow: hidden;
    width: 100% !important;
  }

  .scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding-left: var(--padding);
    padding-right: var(--padding);
    padding-top: 28px;
    width: 100%;
  }

  .scroll > div {
    width: 100% !important;
  }

  .category {
    width: 100%;
  }

  .scroll::-webkit-scrollbar {
    width: 0;
    height: 0;
  }

  .category .sticky {
    display: none;
  }

  .category button .background {
    background-color: rgba(255, 255, 255, 0.1);
  }

  .row {
    justify-content: space-between;
  }

  #nav {
    align-items: center;
    display: flex;
    flex: 0 0 auto;
    justify-content: space-between;
    padding: 8px 24px 16px;
  }

  #nav .bar {
    display: none;
  }

  #nav > .relative {
    justify-content: space-between;
    width: 100%;
  }

  #nav button {
    align-items: center;
    border-radius: 999px;
    color: rgba(var(--em-rgb-color), 0.58);
    display: flex;
    flex: 0 0 40px;
    height: 40px;
    justify-content: center;
    transition:
      background-color var(--duration) var(--easing),
      color var(--duration) var(--easing),
      transform var(--duration) var(--easing);
    width: 40px;
  }

  #nav button:hover,
  #nav button[aria-selected] {
    color: rgb(var(--em-rgb-color));
  }

  #nav button:hover {
    background-color: rgba(var(--em-rgb-color), 0.1);
  }

  #nav button[aria-selected] {
    background-color: rgba(var(--em-rgb-color), 0.14);
  }

  #nav svg,
  #nav img {
    height: 24px;
    width: 24px;
  }
`;

function escapeSvgText(text: string) {
  return text
    .replace(/&/gu, "&amp;")
    .replace(/</gu, "&lt;")
    .replace(/>/gu, "&gt;");
}

function unescapeSvgText(text: string) {
  return text
    .replace(/&gt;/gu, ">")
    .replace(/&lt;/gu, "<")
    .replace(/&amp;/gu, "&");
}

export function emojiAvatarDataUrl(emoji: string, color: string) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512" rx="256" fill="${color}"/><text x="50%" y="56%" dominant-baseline="middle" text-anchor="middle" font-size="${EMOJI_AVATAR_FONT_SIZE}">${escapeSvgText(emoji)}</text></svg>`;
  return `${EMOJI_AVATAR_DATA_URL_PREFIX}${encodeURIComponent(svg)}`;
}

export function parseEmojiAvatarDataUrl(
  avatarUrl: string,
): EmojiAvatarDescriptor | null {
  if (!avatarUrl.startsWith(EMOJI_AVATAR_DATA_URL_PREFIX)) {
    return null;
  }

  try {
    const svg = decodeURIComponent(
      avatarUrl.slice(EMOJI_AVATAR_DATA_URL_PREFIX.length),
    );
    const color = svg.match(/<rect\b[^>]*\sfill="([^"]+)"/u)?.[1];
    const emoji = svg.match(/<text\b[^>]*>(.*?)<\/text>/u)?.[1];

    if (!color || !emoji) {
      return null;
    }

    return { color, emoji: unescapeSvgText(emoji) };
  } catch {
    return null;
  }
}

export function hsvToHex(hue: number, saturation: number, value: number) {
  const normalizedHue = ((hue % 360) + 360) % 360;
  const chroma = (value / 100) * (saturation / 100);
  const huePrime = normalizedHue / 60;
  const secondary = chroma * (1 - Math.abs((huePrime % 2) - 1));
  const match = value / 100 - chroma;
  let red = 0;
  let green = 0;
  let blue = 0;

  if (huePrime >= 0 && huePrime < 1) {
    red = chroma;
    green = secondary;
  } else if (huePrime < 2) {
    red = secondary;
    green = chroma;
  } else if (huePrime < 3) {
    green = chroma;
    blue = secondary;
  } else if (huePrime < 4) {
    green = secondary;
    blue = chroma;
  } else if (huePrime < 5) {
    red = secondary;
    blue = chroma;
  } else {
    red = chroma;
    blue = secondary;
  }

  return [red, green, blue]
    .map((channel) =>
      Math.round((channel + match) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")
    .toUpperCase()
    .padStart(6, "0")
    .replace(/^/, "#");
}

export function hexToHsv(hexColor: string) {
  const match = hexColor.match(/^#?([0-9a-f]{6})$/i);
  if (!match) {
    return {
      hue: DEFAULT_CUSTOM_HUE,
      saturation: DEFAULT_CUSTOM_SATURATION,
      value: DEFAULT_CUSTOM_VALUE,
    };
  }

  const value = match[1];
  const red = Number.parseInt(value.slice(0, 2), 16) / 255;
  const green = Number.parseInt(value.slice(2, 4), 16) / 255;
  const blue = Number.parseInt(value.slice(4, 6), 16) / 255;
  const max = Math.max(red, green, blue);
  const min = Math.min(red, green, blue);
  const delta = max - min;
  let hue = 0;

  if (delta !== 0) {
    if (max === red) {
      hue = 60 * (((green - blue) / delta) % 6);
    } else if (max === green) {
      hue = 60 * ((blue - red) / delta + 2);
    } else {
      hue = 60 * ((red - green) / delta + 4);
    }
  }

  return {
    hue: Math.round((hue + 360) % 360),
    saturation: max === 0 ? 0 : Math.round((delta / max) * 100),
    value: Math.round(max * 100),
  };
}

export function clampPercent(value: number) {
  return Math.max(0, Math.min(100, value));
}

export function snapToGrid(value: number, gridCount: number) {
  if (gridCount <= 1) {
    return clampPercent(value);
  }

  const step = 100 / (gridCount - 1);
  return Math.round(value / step) * step;
}

export function gridInsetPosition(value: number, inset: number) {
  return `calc(${inset}px + (${value} * (100% - ${inset * 2}px) / 100))`;
}

export function hueScrubberPosition(value: number) {
  return `calc(${CUSTOM_HUE_SCRUBBER_INSET}px + (${value} * (100% - ${
    CUSTOM_HUE_SCRUBBER_INSET * 2
  }px) / 100))`;
}

export function normalizeHue(hue: number) {
  return ((hue % 360) + 360) % 360;
}

function hslToRgbString(hslValue: string) {
  const [hue, saturation, lightness] = hslValue
    .trim()
    .split(/\s+/)
    .map((part) => Number.parseFloat(part.replace("%", "")));

  if (
    !Number.isFinite(hue) ||
    !Number.isFinite(saturation) ||
    !Number.isFinite(lightness)
  ) {
    return null;
  }

  const normalizedHue = ((hue % 360) + 360) % 360;
  const saturationRatio = saturation / 100;
  const lightnessRatio = lightness / 100;
  const chroma = (1 - Math.abs(2 * lightnessRatio - 1)) * saturationRatio;
  const huePrime = normalizedHue / 60;
  const secondary = chroma * (1 - Math.abs((huePrime % 2) - 1));
  const match = lightnessRatio - chroma / 2;
  let red = 0;
  let green = 0;
  let blue = 0;

  if (huePrime >= 0 && huePrime < 1) {
    red = chroma;
    green = secondary;
  } else if (huePrime < 2) {
    red = secondary;
    green = chroma;
  } else if (huePrime < 3) {
    green = chroma;
    blue = secondary;
  } else if (huePrime < 4) {
    green = secondary;
    blue = chroma;
  } else if (huePrime < 5) {
    red = secondary;
    blue = chroma;
  } else {
    red = chroma;
    blue = secondary;
  }

  return [red, green, blue]
    .map((channel) => Math.round((channel + match) * 255))
    .join(", ");
}

function hexToRgb(hexColor: string) {
  const match = hexColor.match(/^#?([0-9a-f]{6})$/i);
  if (!match) {
    return null;
  }

  const value = match[1];
  return {
    blue: Number.parseInt(value.slice(4, 6), 16),
    green: Number.parseInt(value.slice(2, 4), 16),
    red: Number.parseInt(value.slice(0, 2), 16),
  };
}

function relativeLuminance(hexColor: string) {
  const rgb = hexToRgb(hexColor);
  if (!rgb) {
    return 0;
  }

  const channels = [rgb.red, rgb.green, rgb.blue].map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.03928
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });

  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}

function contrastRatio(colorA: string, colorB: string) {
  const luminanceA = relativeLuminance(colorA);
  const luminanceB = relativeLuminance(colorB);
  const lighter = Math.max(luminanceA, luminanceB);
  const darker = Math.min(luminanceA, luminanceB);

  return (lighter + 0.05) / (darker + 0.05);
}

export function contrastColorForBackground(backgroundColor: string) {
  return contrastRatio(backgroundColor, "#000000") >=
    contrastRatio(backgroundColor, "#FFFFFF")
    ? "#000000"
    : "#FFFFFF";
}

export function dataTransferHasImage(dataTransfer: DataTransfer | null) {
  if (!dataTransfer) {
    return false;
  }

  const items = Array.from(dataTransfer.items);
  if (items.length > 0) {
    return items.some(
      (item) => item.kind === "file" && item.type.startsWith("image/"),
    );
  }

  return Array.from(dataTransfer.files).some((file) =>
    file.type.startsWith("image/"),
  );
}

export function useEmojiMartStyles(
  containerRef: React.RefObject<HTMLDivElement | null>,
  enabled: boolean,
) {
  React.useEffect(() => {
    if (!enabled) {
      return;
    }

    let animationFrame = 0;

    const installEmojiMartStyles = () => {
      const host = containerRef.current?.querySelector("em-emoji-picker");
      const shadowRoot = host?.shadowRoot;

      if (!shadowRoot) {
        animationFrame = window.requestAnimationFrame(installEmojiMartStyles);
        return;
      }

      if (!shadowRoot.querySelector("#buzz-emoji-mart-style")) {
        const style = document.createElement("style");
        style.id = "buzz-emoji-mart-style";
        style.textContent = EMOJI_MART_SHADOW_CSS;
        shadowRoot.appendChild(style);
      }
    };

    animationFrame = window.requestAnimationFrame(installEmojiMartStyles);

    return () => {
      window.cancelAnimationFrame(animationFrame);
    };
  }, [containerRef, enabled]);
}

export function useEmojiMartThemeVars() {
  const [themeVars, setThemeVars] = React.useState<React.CSSProperties>({});

  React.useEffect(() => {
    const updateThemeVars = () => {
      const styles = window.getComputedStyle(document.documentElement);
      const muted = hslToRgbString(styles.getPropertyValue("--muted"));
      const foreground = hslToRgbString(
        styles.getPropertyValue("--foreground"),
      );
      const background = hslToRgbString(
        styles.getPropertyValue("--background"),
      );

      setThemeVars({
        "--buzz-emoji-picker-rgb-background": muted ?? "54, 58, 79",
        "--buzz-emoji-picker-rgb-color": foreground ?? "245, 247, 255",
        "--buzz-emoji-picker-rgb-input": background ?? "47, 51, 68",
      } as React.CSSProperties);
    };

    updateThemeVars();

    const observer = new MutationObserver(updateThemeVars);
    observer.observe(document.documentElement, {
      attributeFilter: ["class", "style"],
      attributes: true,
    });

    return () => observer.disconnect();
  }, []);

  return themeVars;
}
