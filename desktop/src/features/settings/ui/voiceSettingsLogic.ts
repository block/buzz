export type VoiceAvailability =
  | "bundled"
  | "installed"
  | "downloadable"
  | "unavailable";

export type VoiceRegistryEntry = {
  key: string;
  displayName: string;
  backend: string;
  backendName: string;
  availability: VoiceAvailability;
  fallbackKey: string | null;
  referenceFile: string | null;
  provenance: {
    source: string;
    contentHash: string | null;
    license: string | null;
    sourceUrl: string | null;
  };
};

export function voicesForBackend(
  registry: readonly VoiceRegistryEntry[],
  backend: string,
): VoiceRegistryEntry[] {
  return registry.filter(
    (voice) =>
      voice.backend === backend &&
      (voice.availability === "bundled" || voice.availability === "installed"),
  );
}

export function selectedVoiceForBackend(
  preferences: readonly string[],
  voices: readonly VoiceRegistryEntry[],
): VoiceRegistryEntry | undefined {
  for (const key of preferences) {
    const voice = voices.find((candidate) => candidate.key === key);
    if (voice) return voice;
  }
  return voices.find((voice) => voice.fallbackKey === null) ?? voices[0];
}

export function voiceOptionLabel(
  voice: VoiceRegistryEntry,
  voices: readonly VoiceRegistryEntry[],
): string {
  const duplicateLabel = voices.some(
    (candidate) =>
      candidate.key !== voice.key &&
      candidate.displayName === voice.displayName,
  );
  if (!duplicateLabel) return voice.displayName;
  const identitySuffix = voice.key.split(":").at(-1)?.slice(-8) ?? voice.key;
  return `${voice.displayName} · ${identitySuffix}`;
}
