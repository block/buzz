export interface ParticleGridSettings {
  gridSize: number;
  charScale: number;
  noiseScale: number;
  noiseSpeed: number;
  idleOpacity: number;
  baseOpacity: number;
  touchRadius: number;
  touchStrength: number;
  trailDuration: number;
  charGamma: number;
  dotColor: [number, number, number];
  backgroundColor: [number, number, number];
  mode: number;
  gridOpacity: number;
  thinkingFade: number;
  pullProgress: number;
  audioLevel: number;
  audioPhase: number;
  showSparkles: boolean;
  charSet: string;
  objectRevealStyle: number;
}

export function inverseParticleColor([red, green, blue]: [
  number,
  number,
  number,
]): [number, number, number] {
  return [1 - red, 1 - green, 1 - blue];
}

/**
 * The Simon Canvas "Thinking" preset, adapted as the stable starting point for
 * Buzz's custom-card canvas. Keeping the preset in one exported factory makes
 * later visual tuning explicit without coupling it to the React renderer.
 */
export function createDefaultSettings(): ParticleGridSettings {
  const backgroundColor: [number, number, number] = [1, 1, 1];
  return {
    gridSize: 15,
    charScale: 1,
    noiseScale: 0.0008,
    noiseSpeed: 0.5,
    idleOpacity: 0.14,
    baseOpacity: 0.3,
    touchRadius: 1175,
    touchStrength: 1.25,
    trailDuration: 4,
    charGamma: 1,
    dotColor: inverseParticleColor(backgroundColor),
    backgroundColor,
    mode: 2,
    gridOpacity: 1,
    thinkingFade: 1,
    pullProgress: 0,
    audioLevel: 0,
    audioPhase: 0,
    showSparkles: false,
    charSet: "H",
    objectRevealStyle: 5,
  };
}
