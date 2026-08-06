export type ArtillerySoundCue = "launch" | "impact" | "victory";

type WebkitAudioWindow = typeof window & {
  webkitAudioContext?: typeof AudioContext;
};

let audioContext: AudioContext | null = null;
let masterGain: GainNode | null = null;
let enabled = true;
let activeWhistle: {
  airGain: GainNode;
  source: AudioBufferSourceNode;
  whistleGain: GainNode;
} | null = null;
let activeRavineYell: (() => void) | null = null;

const ARTILLERY_VOLUME_BOOST = 1.2;

function getAudioContext() {
  if (audioContext) return audioContext;
  const AudioContextClass =
    window.AudioContext ?? (window as WebkitAudioWindow).webkitAudioContext;
  if (!AudioContextClass) return null;
  audioContext = new AudioContextClass();
  return audioContext;
}

function connectGain(context: AudioContext, volume: number) {
  if (!masterGain) {
    masterGain = context.createGain();
    masterGain.gain.value = ARTILLERY_VOLUME_BOOST;
    masterGain.connect(context.destination);
  }
  const gain = context.createGain();
  gain.gain.value = volume;
  gain.connect(masterGain);
  return gain;
}

/** Stops the continuous in-flight whistle with a short click-free fade. */
export function stopArtilleryWhistle() {
  if (!activeWhistle || !audioContext) return;
  const { airGain, source, whistleGain } = activeWhistle;
  activeWhistle = null;
  const now = audioContext.currentTime;
  for (const gain of [airGain, whistleGain]) {
    gain.gain.cancelScheduledValues(now);
    gain.gain.setValueAtTime(Math.max(gain.gain.value, 0.0001), now);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.045);
  }
  source.stop(now + 0.055);
}

/** Starts an aerodynamic shell rush and whistle for the flight duration. */
export function startArtilleryWhistle(durationMs: number) {
  if (!enabled) return;
  const context = getAudioContext();
  if (context?.state !== "running") return;
  stopArtilleryWhistle();

  const now = context.currentTime;
  const duration = Math.max(0.12, durationMs / 1_000);
  const fadeOutAt = now + Math.max(0.07, duration - 0.07);

  const whistleGain = connectGain(context, 0.0001);
  whistleGain.gain.setValueAtTime(0.0001, now);
  whistleGain.gain.exponentialRampToValueAtTime(0.052, now + 0.055);
  whistleGain.gain.setValueAtTime(0.052, fadeOutAt);
  whistleGain.gain.exponentialRampToValueAtTime(0.0001, now + duration);
  const whistleFilter = context.createBiquadFilter();
  whistleFilter.type = "bandpass";
  whistleFilter.Q.value = 14;
  whistleFilter.frequency.setValueAtTime(1_850, now);
  whistleFilter.frequency.exponentialRampToValueAtTime(
    1_050,
    now + duration * 0.55,
  );
  whistleFilter.frequency.exponentialRampToValueAtTime(2_250, now + duration);
  whistleFilter.connect(whistleGain);

  const airGain = connectGain(context, 0.0001);
  airGain.gain.setValueAtTime(0.0001, now);
  airGain.gain.exponentialRampToValueAtTime(0.022, now + 0.035);
  airGain.gain.linearRampToValueAtTime(0.036, fadeOutAt);
  airGain.gain.exponentialRampToValueAtTime(0.0001, now + duration);
  const airFilter = context.createBiquadFilter();
  airFilter.type = "bandpass";
  airFilter.Q.value = 0.75;
  airFilter.frequency.setValueAtTime(720, now);
  airFilter.frequency.exponentialRampToValueAtTime(1_450, now + duration);
  airFilter.connect(airGain);

  const source = createNoise(context, duration + 0.08);
  source.connect(whistleFilter);
  source.connect(airFilter);
  source.start(now);
  source.stop(now + duration + 0.02);
  activeWhistle = { airGain, source, whistleGain };
}

function playLaunch(context: AudioContext) {
  const now = context.currentTime;
  const gain = connectGain(context, 0.16);
  gain.gain.setValueAtTime(0.0001, now);
  gain.gain.exponentialRampToValueAtTime(0.16, now + 0.018);
  gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.42);

  const oscillator = context.createOscillator();
  oscillator.type = "sawtooth";
  oscillator.frequency.setValueAtTime(210, now);
  oscillator.frequency.exponentialRampToValueAtTime(58, now + 0.4);
  oscillator.connect(gain);
  oscillator.start(now);
  oscillator.stop(now + 0.43);
}

function createNoise(context: AudioContext, duration: number) {
  const buffer = context.createBuffer(
    1,
    Math.ceil(context.sampleRate * duration),
    context.sampleRate,
  );
  const data = buffer.getChannelData(0);
  for (let index = 0; index < data.length; index += 1) {
    data[index] = Math.random() * 2 - 1;
  }
  const source = context.createBufferSource();
  source.buffer = buffer;
  return source;
}

function playImpact(context: AudioContext) {
  const now = context.currentTime;
  const noiseGain = connectGain(context, 0.24);
  noiseGain.gain.setValueAtTime(0.24, now);
  noiseGain.gain.exponentialRampToValueAtTime(0.0001, now + 0.48);
  const filter = context.createBiquadFilter();
  filter.type = "lowpass";
  filter.frequency.setValueAtTime(1_400, now);
  filter.frequency.exponentialRampToValueAtTime(180, now + 0.45);
  filter.connect(noiseGain);
  const noise = createNoise(context, 0.5);
  noise.connect(filter);
  noise.start(now);

  const boomGain = connectGain(context, 0.2);
  boomGain.gain.setValueAtTime(0.2, now);
  boomGain.gain.exponentialRampToValueAtTime(0.0001, now + 0.6);
  const boom = context.createOscillator();
  boom.type = "sine";
  boom.frequency.setValueAtTime(95, now);
  boom.frequency.exponentialRampToValueAtTime(34, now + 0.55);
  boom.connect(boomGain);
  boom.start(now);
  boom.stop(now + 0.62);
}

function playVictory(context: AudioContext) {
  const now = context.currentTime;
  const notes = [261.63, 329.63, 392, 523.25];
  for (const [index, frequency] of notes.entries()) {
    const start = now + index * 0.11;
    const gain = connectGain(context, 0.09);
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.09, start + 0.025);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.42);
    const oscillator = context.createOscillator();
    oscillator.type = "triangle";
    oscillator.frequency.value = frequency;
    oscillator.connect(gain);
    oscillator.start(start);
    oscillator.stop(start + 0.44);
  }
}

/** Starts a loud, descending synthesized yell for the ravine fall. */
export function startArtilleryRavineYell() {
  if (!enabled) return () => {};
  const context = getAudioContext();
  if (context?.state !== "running") return () => {};
  activeRavineYell?.();

  const now = context.currentTime;
  const duration = 1.85;
  const voiceGain = connectGain(context, 0.0001);
  voiceGain.gain.setValueAtTime(0.0001, now);
  voiceGain.gain.exponentialRampToValueAtTime(0.24, now + 0.035);
  voiceGain.gain.linearRampToValueAtTime(0.2, now + 1.15);
  voiceGain.gain.exponentialRampToValueAtTime(0.0001, now + duration);

  const formant = context.createBiquadFilter();
  formant.type = "bandpass";
  formant.Q.value = 1.8;
  formant.frequency.setValueAtTime(1_050, now);
  formant.frequency.exponentialRampToValueAtTime(620, now + duration);
  formant.connect(voiceGain);

  const voices = [-9, 9].map((detune) => {
    const oscillator = context.createOscillator();
    oscillator.type = "sawtooth";
    oscillator.detune.value = detune;
    oscillator.frequency.setValueAtTime(510, now);
    oscillator.frequency.exponentialRampToValueAtTime(145, now + duration);
    oscillator.connect(formant);
    oscillator.start(now);
    oscillator.stop(now + duration + 0.05);
    return oscillator;
  });

  const vibrato = context.createOscillator();
  const vibratoDepth = context.createGain();
  vibrato.type = "sine";
  vibrato.frequency.setValueAtTime(7.5, now);
  vibrato.frequency.linearRampToValueAtTime(11, now + duration);
  vibratoDepth.gain.setValueAtTime(18, now);
  vibratoDepth.gain.linearRampToValueAtTime(8, now + duration);
  vibrato.connect(vibratoDepth);
  for (const voice of voices) vibratoDepth.connect(voice.frequency);
  vibrato.start(now);
  vibrato.stop(now + duration + 0.05);

  const breathGain = connectGain(context, 0.035);
  breathGain.gain.setValueAtTime(0.0001, now);
  breathGain.gain.exponentialRampToValueAtTime(0.035, now + 0.025);
  breathGain.gain.exponentialRampToValueAtTime(0.0001, now + duration);
  const breathFilter = context.createBiquadFilter();
  breathFilter.type = "bandpass";
  breathFilter.Q.value = 0.7;
  breathFilter.frequency.value = 1_300;
  breathFilter.connect(breathGain);
  const breath = createNoise(context, duration + 0.06);
  breath.connect(breathFilter);
  breath.start(now);
  breath.stop(now + duration + 0.05);

  let stopped = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    const stopAt = context.currentTime;
    for (const gain of [voiceGain, breathGain]) {
      gain.gain.cancelScheduledValues(stopAt);
      gain.gain.setValueAtTime(Math.max(gain.gain.value, 0.0001), stopAt);
      gain.gain.exponentialRampToValueAtTime(0.0001, stopAt + 0.055);
    }
    for (const voice of voices) voice.stop(stopAt + 0.065);
    vibrato.stop(stopAt + 0.065);
    breath.stop(stopAt + 0.065);
    if (activeRavineYell === stop) activeRavineYell = null;
  };
  activeRavineYell = stop;
  window.setTimeout(
    () => {
      if (activeRavineYell === stop) activeRavineYell = null;
    },
    (duration + 0.1) * 1_000,
  );
  return stop;
}

/** Resumes Web Audio from a user gesture when autoplay policy requires it. */
export async function unlockArtilleryAudio() {
  if (!enabled) return;
  const context = getAudioContext();
  if (context?.state === "suspended") await context.resume().catch(() => {});
}

/** Plays one arena cue; unavailable or locked audio fails silently. */
export function playArtillerySound(cue: ArtillerySoundCue) {
  if (!enabled) return;
  const context = getAudioContext();
  if (context?.state !== "running") return;
  if (cue === "launch") playLaunch(context);
  else if (cue === "impact") playImpact(context);
  else playVictory(context);
}

export function setArtilleryAudioEnabled(nextEnabled: boolean) {
  enabled = nextEnabled;
  if (enabled) void unlockArtilleryAudio();
  else {
    stopArtilleryWhistle();
    activeRavineYell?.();
  }
}

export function isArtilleryAudioEnabled() {
  return enabled;
}
