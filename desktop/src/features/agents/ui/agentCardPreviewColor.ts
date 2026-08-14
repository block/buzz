type ColorAccumulator = {
  blue: number;
  green: number;
  red: number;
  weight: number;
};

type ColorSample = {
  blue: number;
  green: number;
  hue: number;
  red: number;
  weight: number;
};

const HUE_BIN_COUNT = 24;
const HUE_CLUSTER_RADIUS = 2;

const EMPTY_ACCUMULATOR: ColorAccumulator = {
  blue: 0,
  green: 0,
  red: 0,
  weight: 0,
};

/**
 * Pick a representative avatar color for the trading-card foil.
 *
 * Colorful pixels are preferred so a neutral background does not wash out a
 * distinctive avatar. Among those pixels, choose the hue family covering the
 * most visual area before averaging it, so a small vivid accent cannot
 * overpower the avatar's dominant color. Fully transparent and
 * near-black/near-white pixels are ignored; if an avatar is intentionally
 * monochrome, the neutral sample is used as the fallback.
 */
export function sampleRepresentativeAvatarColor(
  pixels: Uint8ClampedArray,
): string | null {
  if (pixels.length < 4 || pixels.length % 4 !== 0) return null;

  const colorfulSamples: ColorSample[] = [];
  const hueWeights = Array<number>(HUE_BIN_COUNT).fill(0);
  const neutral = { ...EMPTY_ACCUMULATOR };

  for (let offset = 0; offset < pixels.length; offset += 4) {
    const red = pixels[offset] ?? 0;
    const green = pixels[offset + 1] ?? 0;
    const blue = pixels[offset + 2] ?? 0;
    const alpha = (pixels[offset + 3] ?? 0) / 255;
    if (alpha < 0.25) continue;

    const maximum = Math.max(red, green, blue) / 255;
    const minimum = Math.min(red, green, blue) / 255;
    const lightness = (maximum + minimum) / 2;
    if (lightness < 0.07 || lightness > 0.95) continue;

    const chroma = maximum - minimum;
    const saturation =
      chroma === 0 ? 0 : chroma / (1 - Math.abs(2 * lightness - 1));
    const midtoneWeight = 1 - Math.abs(lightness - 0.5) * 0.9;
    const baseWeight = alpha * midtoneWeight;

    accumulate(neutral, red, green, blue, baseWeight);
    if (saturation >= 0.18) {
      const hue = rgbHue(red / 255, green / 255, blue / 255, maximum, minimum);
      colorfulSamples.push({ blue, green, hue, red, weight: baseWeight });
      hueWeights[Math.floor((hue / 360) * HUE_BIN_COUNT) % HUE_BIN_COUNT] +=
        baseWeight;
    }
  }

  if (colorfulSamples.length === 0) return formatAccumulator(neutral);

  const dominantBin = hueWeights.reduce(
    (bestBin, _, bin) =>
      clusteredHueWeight(hueWeights, bin) >
      clusteredHueWeight(hueWeights, bestBin)
        ? bin
        : bestBin,
    0,
  );
  const dominantHue = ((dominantBin + 0.5) / HUE_BIN_COUNT) * 360;
  const maximumHueDistance = ((HUE_CLUSTER_RADIUS + 0.5) / HUE_BIN_COUNT) * 360;
  const dominant = { ...EMPTY_ACCUMULATOR };
  for (const sample of colorfulSamples) {
    if (circularHueDistance(sample.hue, dominantHue) <= maximumHueDistance) {
      accumulate(
        dominant,
        sample.red,
        sample.green,
        sample.blue,
        sample.weight,
      );
    }
  }

  return formatAccumulator(dominant.weight > 0 ? dominant : neutral);
}

export function fallbackAgentCardColor(seed: string): string {
  let hash = 0;
  for (const character of seed) {
    hash = (hash * 31 + (character.codePointAt(0) ?? 0)) >>> 0;
  }
  return `hsl(${hash % 360} 68% 56%)`;
}

function accumulate(
  accumulator: ColorAccumulator,
  red: number,
  green: number,
  blue: number,
  weight: number,
) {
  accumulator.red += red * weight;
  accumulator.green += green * weight;
  accumulator.blue += blue * weight;
  accumulator.weight += weight;
}

function clusteredHueWeight(hueWeights: number[], centerBin: number): number {
  let weight = 0;
  for (
    let offset = -HUE_CLUSTER_RADIUS;
    offset <= HUE_CLUSTER_RADIUS;
    offset += 1
  ) {
    const bin = (centerBin + offset + HUE_BIN_COUNT) % HUE_BIN_COUNT;
    weight += hueWeights[bin] ?? 0;
  }
  return weight;
}

function circularHueDistance(first: number, second: number): number {
  const distance = Math.abs(first - second);
  return Math.min(distance, 360 - distance);
}

function rgbHue(
  red: number,
  green: number,
  blue: number,
  maximum: number,
  minimum: number,
): number {
  const chroma = maximum - minimum;
  if (chroma === 0) return 0;
  const hue =
    maximum === red
      ? ((green - blue) / chroma) % 6
      : maximum === green
        ? (blue - red) / chroma + 2
        : (red - green) / chroma + 4;
  return (((hue * 60) % 360) + 360) % 360;
}

function formatAccumulator(accumulator: ColorAccumulator): string | null {
  if (accumulator.weight === 0) return null;
  return `rgb(${Math.round(accumulator.red / accumulator.weight)} ${Math.round(accumulator.green / accumulator.weight)} ${Math.round(accumulator.blue / accumulator.weight)})`;
}
